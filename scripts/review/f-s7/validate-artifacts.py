#!/usr/bin/env python3
"""Validate closure-critical F-S7 artifact JSON invariants."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
from pathlib import Path
from typing import Any


HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
TOPOLOGIES = ("MoeTiny", "MoeTinyDenseMatched")
SEEDS = range(5)
D11_GRID = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0]
PRODUCTION_SWEEP_PRODUCER_KIND = "production_closure_retrain_score"
S7_N_BLOCKS = 4
S7_N_EXPERTS = 4
S7_OPTIMIZER_STEPS = 20_000
S7_EVAL_EVERY_STEPS = 1_000
FLOAT_TOL = 1.0e-9
S7_PARITY_BPC_MARGIN = 0.05
PARETO_VERDICTS = {
    "MoE-dominates",
    "dense-dominates",
    "MoE-wins-under-byte-equivalence",
    "Dense-wins-under-byte-equivalence",
    "Incomparable",
    "Tied",
}
Domain = tuple[str, str, str, str]
SELF_HASH_CACHE: dict[tuple[Domain, str], str] = {}
RUN_LOG_DOMAIN: Domain = ("gbf-artifact", "S7RunLog", "s7_run_log.v1", "1")
RAW_LOSS_DIAGNOSTICS_DOMAIN: Domain = (
    "gbf-artifact",
    "RawLossDiagnostics",
    "s7_raw_loss_diagnostics.v1",
    "1",
)
SCORE_DOMAIN: Domain = ("gbf-artifact", "S7ScoreReport", "s7_score.v1", "1")
SWITCH_STATS_DOMAIN: Domain = (
    "gbf-experiments",
    "S7SwitchStatsReport",
    "s7_switch_stats.v1",
    "1",
)
TEMPORAL_SWITCH_DIGEST_DOMAIN: Domain = (
    "gbf-artifact",
    "TemporalSwitchDigest",
    "s7_temporal_switch_digest.v1",
    "1",
)
EXPERT_SLOT_AFFINITY_DOMAIN: Domain = (
    "gbf-artifact",
    "ExpertSlotAffinity",
    "s7_expert_slot_affinity.v1",
    "1",
)
CLIP_SATURATION_DIGEST_DOMAIN: Domain = (
    "gbf-artifact",
    "ClipSaturationDigest",
    "s7_clip_saturation_digest.v1",
    "1",
)
EXPERT_PAYLOAD_DIGEST_DOMAIN: Domain = (
    "gbf-artifact",
    "ExpertPayloadDigest",
    "s7_expert_payload_digest.v1",
    "1",
)
MATCHED_BYTES_PIN_DOMAIN: Domain = (
    "gbf-artifact",
    "MatchedBytesPin",
    "s7_matched_bytes_pin.v1",
    "1",
)
DENSE_VS_MOE_DOMAIN: Domain = (
    "gbf-artifact",
    "S7DenseVsMoeComparisonReport",
    "s7_dense_vs_moe.v1",
    "1",
)
LAMBDA_SWITCH_RECORD_DOMAIN: Domain = (
    "gbf-experiments",
    "LambdaSwitchSweepRecord",
    "s7_lambda_switch_sweep_step.v1",
    "1",
)
ROUTER_COLLAPSE_SWEEP_DOMAIN: Domain = (
    "gbf-experiments",
    "RouterCollapseSweepReport",
    "s7_router_collapse_sweep.v1",
    "1",
)
FRONTIER_DOMAIN: Domain = (
    "gbf-experiments",
    "S7FrontierReport",
    "s7_frontier.v1",
    "1",
)
BURN_GRAD_SMOKE_DOMAIN: Domain = (
    "gbf-experiments",
    "S7BurnGradSmokeReport",
    "s7_burn_grad_smoke.v1",
    "1",
)
ORACLE_ROUTED_DOMAIN: Domain = (
    "gbf-experiments",
    "S7OracleRoutedReport",
    "s7_oracle_routed.v1",
    "1",
)
ROUTER_STEP_TELEMETRY_DOMAIN: Domain = (
    "gbf-experiments",
    "RouterStepTelemetry",
    "s7_router_step_telemetry.v1",
    "1",
)
EMULATOR_ONE_TOKEN_DOMAIN: Domain = (
    "gbf-experiments",
    "EmulatorOneTokenReport",
    "s7_emulator_one_token.v1",
    "1",
)
ROUTER_STEP_TELEMETRY_FIELDS = {
    "schema_version",
    "seed",
    "train_step",
    "layer_id",
    "expert_usage_entropy_bits",
    "same_expert_rate",
    "router_confidence_distribution",
    "tokens_per_expert",
    "bank_switches_per_token",
    "telemetry_self_hash",
}


class DuplicateKeyError(ValueError):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate production F-S7 artifact JSON shape and key closure invariants."
    )
    parser.add_argument("--root", default=".", help="repository root or packet root")
    parser.add_argument(
        "--fast-fail",
        action="store_true",
        help="return after the first validation phase with errors; used by focused regression tests",
    )
    parser.add_argument(
        "--self-test-optimizer-steps",
        type=int,
        help=(
            "test-only override for verify-packet --self-test synthetic fixtures; "
            "production closure checks must use the default 20000 steps"
        ),
    )
    parser.add_argument(
        "--self-test-eval-every-steps",
        type=int,
        help=(
            "test-only eval cadence override paired with --self-test-optimizer-steps"
        ),
    )
    args = parser.parse_args()

    config_errors = configure_self_test_step_counts(
        args.self_test_optimizer_steps,
        args.self_test_eval_every_steps,
    )
    if config_errors:
        print("S7 artifact closure shape: NEEDS_CHANGES")
        for error in config_errors:
            print(f" - {error}")
        return 1

    root = Path(args.root)
    errors = validate_packet(root, fast_fail=args.fast_fail)
    if errors:
        print("S7 artifact closure shape: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1
    print("S7 artifact closure shape: ok")
    return 0


def configure_self_test_step_counts(
    optimizer_steps: int | None, eval_every_steps: int | None
) -> list[str]:
    global S7_OPTIMIZER_STEPS, S7_EVAL_EVERY_STEPS
    if optimizer_steps is None and eval_every_steps is None:
        return []
    if optimizer_steps is None or eval_every_steps is None:
        return [
            "--self-test-optimizer-steps and --self-test-eval-every-steps must be passed together"
        ]
    if optimizer_steps <= 0:
        return ["--self-test-optimizer-steps must be positive"]
    if eval_every_steps <= 0:
        return ["--self-test-eval-every-steps must be positive"]
    if optimizer_steps % eval_every_steps != 0:
        return ["--self-test-optimizer-steps must be divisible by --self-test-eval-every-steps"]
    S7_OPTIMIZER_STEPS = optimizer_steps
    S7_EVAL_EVERY_STEPS = eval_every_steps
    return []


def validate_packet(root: Path, fast_fail: bool = False) -> list[str]:
    errors: list[str] = []
    for topology in TOPOLOGIES:
        for seed in SEEDS:
            validate_score(
                errors,
                root / f"experiments/S7/scores/{topology}/seed-{seed}/score.json",
                topology,
                seed,
            )
            if fast_fail and errors:
                return errors
    for seed in SEEDS:
        validate_switch_stats(
            errors,
            root / f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json",
            seed,
        )
        if fast_fail and errors:
            return errors
    validate_sweep(errors, root / "experiments/S7/router-collapse/seed-0/sweep.json")
    if fast_fail and errors:
        return errors
    validate_dense_vs_moe(errors, root / "experiments/S7/dense-vs-moe/comparison.json")
    if fast_fail and errors:
        return errors
    validate_frontier(errors, root / "experiments/S7/frontier/frontier.json")
    if fast_fail and errors:
        return errors
    validate_burn_grad(errors, root / "experiments/S7/burn-grad-smoke/expert_block_qat.json")
    if fast_fail and errors:
        return errors
    validate_oracle(errors, root / "experiments/S7/oracle-routed/seed-0/oracle.json")
    if fast_fail and errors:
        return errors
    validate_emulator(
        errors,
        root / "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
        "MoeTiny",
    )
    if fast_fail and errors:
        return errors
    dense_emulator = root / "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json"
    if dense_emulator.exists():
        validate_emulator(errors, dense_emulator, "MoeTinyDenseMatched")
        if fast_fail and errors:
            return errors
    for topology in TOPOLOGIES:
        for seed in SEEDS:
            run_log = validate_run_log(
                errors,
                root / f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json",
                topology,
                seed,
            )
            if fast_fail and errors:
                return errors
            validate_grad_log(
                errors,
                root / f"experiments/S7/runs/{topology}/seed-{seed}/grad-log.jsonl",
                seed,
                run_log.get("grad_norms") if run_log is not None else None,
            )
            if fast_fail and errors:
                return errors
            validate_router_step_telemetry_log(
                errors,
                root / f"experiments/S7/runs/{topology}/seed-{seed}/router-step-telemetry.jsonl",
                topology,
                seed,
            )
            if fast_fail and errors:
                return errors
    return errors


def load_json(errors: list[str], path: Path, schema: str) -> dict[str, Any] | None:
    if not path.is_file():
        errors.append(f"missing {schema}: {path}")
        return None
    text = path.read_text(encoding="utf-8")
    try:
        payload = json.loads(text, object_pairs_hook=reject_duplicate_keys)
    except DuplicateKeyError as error:
        errors.append(f"{path} has duplicate JSON key: {error}")
        return None
    except json.JSONDecodeError as error:
        errors.append(f"{path} is not valid JSON: {error}")
        return None
    if not isinstance(payload, dict):
        errors.append(f"{path} must contain a JSON object")
        return None
    try:
        canonical = canonical_json_text(payload)
    except (TypeError, ValueError) as error:
        errors.append(f"{path} has non-canonical JSON value: {error}")
        return None
    if text not in {canonical, f"{canonical}\n"}:
        errors.append(f"{path} must use canonical JSON bytes: sorted keys, compact UTF-8, no insignificant whitespace")
    observed = payload.get("schema")
    if observed != schema:
        errors.append(f"{path} schema must be {schema}, observed {observed!r}")
    return payload


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, value in pairs:
        if key in out:
            raise DuplicateKeyError(key)
        out[key] = value
    return out


def canonical_json_text(payload: Any) -> str:
    if payload is None:
        return "null"
    if payload is True:
        return "true"
    if payload is False:
        return "false"
    if isinstance(payload, int) and not isinstance(payload, bool):
        return str(payload)
    if isinstance(payload, float):
        if not math.isfinite(payload):
            raise ValueError("non-finite float in canonical JSON payload")
        if payload == 0.0:
            return "0.0"
        return json.dumps(payload, allow_nan=False).replace("e+", "e")
    if isinstance(payload, str):
        return json.dumps(payload, ensure_ascii=False, allow_nan=False)
    if isinstance(payload, list):
        return "[" + ",".join(canonical_json_text(item) for item in payload) + "]"
    if isinstance(payload, dict):
        return (
            "{"
            + ",".join(
                f"{json.dumps(key, ensure_ascii=False, allow_nan=False)}:{canonical_json_text(payload[key])}"
                for key in sorted(payload)
            )
            + "}"
        )
    raise TypeError(f"unsupported JSON value for canonical encoding: {type(payload).__name__}")


def domain_self_hash(domain: Domain, payload_without_self_hash: dict[str, Any]) -> str:
    canonical = canonical_json_text(payload_without_self_hash)
    cache_key = (domain, canonical)
    cached = SELF_HASH_CACHE.get(cache_key)
    if cached is not None:
        return cached
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + canonical.encode("utf-8")
    )
    digest = f"sha256:{hashlib.sha256(material).hexdigest()}"
    SELF_HASH_CACHE[cache_key] = digest
    return digest


def verify_domain_self_hash(
    errors: list[str],
    location: str,
    data: dict[str, Any],
    field: str,
    domain: Domain,
) -> None:
    observed = data.get(field)
    if not is_hash(observed):
        errors.append(f"{location} {field} self-hash missing or malformed")
        return
    if not field.endswith("_self_hash"):
        errors.append(f"{location} internal error: {field} is not a self-hash field")
        return
    payload = {key: value for key, value in data.items() if key != field}
    remaining = sorted(key for key in payload if key.endswith("_self_hash"))
    if remaining:
        errors.append(
            f"{location} {field} self-hash input leaves top-level self-hash fields: {', '.join(remaining)}"
        )
        return
    try:
        expected = domain_self_hash(domain, payload)
    except (TypeError, ValueError) as error:
        errors.append(f"{location} {field} self-hash input has non-canonical JSON value: {error}")
        return
    if observed != expected:
        errors.append(f"{location} {field} self-hash mismatch: expected {expected}, observed {observed}")


def version_dict() -> dict[str, int]:
    return {"major": 1, "minor": 0, "patch": 0}


def validate_run_log(errors: list[str], path: Path, topology: str, seed: int) -> dict[str, Any] | None:
    data = load_json(errors, path, "s7_run_log.v1")
    if data is None:
        return None
    require_equal(errors, path, data, "seed", seed)
    require_equal(errors, path, data, "topology", topology)
    require_hash(errors, path, data, "train_config_hash")
    require_hash(errors, path, data, "model_topology_hash")
    require_hash(errors, path, data, "loss_config_hash")
    require_hash(errors, path, data, "phase_schedule_hash")
    require_hash(errors, path, data, "frozen_teacher_checkpoint_sha")
    require_hash(errors, path, data, "run_log_self_hash")
    if topology == "MoeTinyDenseMatched":
        if data.get("router_config_hash") is not None:
            errors.append(f"{path} dense router_config_hash must be null")
        if data.get("expert_block_config_hash") is not None:
            errors.append(f"{path} dense expert_block_config_hash must be null")
    else:
        require_hash(errors, path, data, "router_config_hash")
        require_hash(errors, path, data, "expert_block_config_hash")
    validate_run_log_completion(errors, path, data)
    validate_run_log_series(errors, path, data)
    validate_run_log_grad_norms(errors, path, data)
    verify_domain_self_hash(errors, str(path), data, "run_log_self_hash", RUN_LOG_DOMAIN)
    return data


def validate_run_log_completion(errors: list[str], path: Path, data: dict[str, Any]) -> None:
    completion = data.get("completion")
    if completion != {"kind": "completed"}:
        errors.append(
            f"{path} completion must be Rust tagged S7Completion {{\"kind\":\"completed\"}} for closure"
        )


def validate_run_log_series(errors: list[str], path: Path, data: dict[str, Any]) -> None:
    losses = data.get("losses")
    grad_norms = data.get("grad_norms")
    eval_points = data.get("eval_points")
    if not isinstance(losses, list):
        errors.append(f"{path} losses must be a list")
        losses = []
    if not isinstance(grad_norms, list):
        errors.append(f"{path} grad_norms must be a list")
        grad_norms = []
    if not isinstance(eval_points, list):
        errors.append(f"{path} eval_points must be a list")
        eval_points = []
    if len(losses) != S7_OPTIMIZER_STEPS:
        errors.append(f"{path} losses length must be {S7_OPTIMIZER_STEPS} for completed run")
    if len(grad_norms) != len(losses):
        errors.append(f"{path} grad_norms length must match losses length")
    expected_eval_points = (S7_OPTIMIZER_STEPS // S7_EVAL_EVERY_STEPS) + 1
    if len(eval_points) != expected_eval_points:
        errors.append(f"{path} eval_points length must be {expected_eval_points} for completed run")
    for index, entry in enumerate(losses):
        expected_step = index + 1
        location = f"{path} losses[{index}]"
        if not isinstance(entry, list) or len(entry) != 2:
            errors.append(f"{location} must be [train_step, RawLossDiagnostics]")
            continue
        step, diagnostics = entry
        if step != expected_step:
            errors.append(f"{location} train_step must be {expected_step}")
        validate_raw_loss_diagnostics(errors, location, diagnostics)
    for index, entry in enumerate(eval_points):
        expected_step = index * S7_EVAL_EVERY_STEPS
        location = f"{path} eval_points[{index}]"
        if not isinstance(entry, list) or len(entry) != 2:
            errors.append(f"{location} must be [eval_step, bpc]")
            continue
        step, bpc = entry
        if step != expected_step:
            errors.append(f"{location} eval_step must be {expected_step}")
        if not finite_number(bpc):
            errors.append(f"{location} bpc must be finite")


def validate_raw_loss_diagnostics(errors: list[str], location: str, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{location} must contain a RawLossDiagnostics object")
        return
    expected_fields = {
        "lm_loss_raw",
        "distill_loss_raw",
        "balance_loss_raw",
        "zrouter_loss_raw",
        "switch_loss_raw",
        "diagnostics_self_hash",
    }
    observed_fields = set(value)
    if observed_fields != expected_fields:
        errors.append(
            f"{location} RawLossDiagnostics fields must be lm_loss_raw, distill_loss_raw, balance_loss_raw, zrouter_loss_raw, switch_loss_raw, diagnostics_self_hash"
        )
        return
    for field in ["lm_loss_raw", "balance_loss_raw", "zrouter_loss_raw"]:
        if not finite_number(value.get(field)) or float(value[field]) < 0.0:
            errors.append(f"{location} {field} must be finite and non-negative")
    if not unit_interval(value.get("switch_loss_raw")):
        errors.append(f"{location} switch_loss_raw must be finite and in [0, 1]")
    validate_distill_raw_diagnostic(errors, f"{location} distill_loss_raw", value["distill_loss_raw"])
    verify_domain_self_hash(
        errors,
        location,
        value,
        "diagnostics_self_hash",
        RAW_LOSS_DIAGNOSTICS_DOMAIN,
    )


def validate_distill_raw_diagnostic(errors: list[str], location: str, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{location} must be a DistillRawDiagnostic object")
        return
    kind = value.get("kind")
    if kind == "not_available":
        if set(value) != {"kind", "reason", "phase"}:
            errors.append(f"{location} not_available fields must be kind, reason, phase")
            return
        if not non_empty_string(value.get("reason")):
            errors.append(f"{location} reason must be a non-empty string")
        if value.get("phase") not in {"phase_a", "phase_b", "phase_c", "phase_d", "phase_e"}:
            errors.append(f"{location} phase must be a TrainPhase snake_case tag")
    elif kind == "value":
        if set(value) != {"kind", "loss"}:
            errors.append(f"{location} value fields must be kind, loss")
            return
        if not finite_number(value.get("loss")) or float(value["loss"]) < 0.0:
            errors.append(f"{location} loss must be finite and non-negative")
    else:
        errors.append(f"{location} kind must be not_available or value")


def validate_run_log_grad_norms(errors: list[str], path: Path, data: dict[str, Any]) -> None:
    grad_norms = data.get("grad_norms")
    if isinstance(grad_norms, list):
        for index, entry in enumerate(grad_norms):
            expected_step = index + 1
            location = f"{path} grad_norms[{index}]"
            if not isinstance(entry, list) or len(entry) != 2:
                errors.append(f"{location} must be [train_step, GradNormSummary]")
                continue
            step, summary = entry
            if step != expected_step:
                errors.append(f"{location} train_step must match loss step {expected_step}")
            validate_grad_norm_summary(errors, location, summary)
    final_grad_norms = data.get("final_grad_norms")
    if final_grad_norms is None:
        errors.append(f"{path} missing final_grad_norms")
    else:
        validate_grad_norm_summary(errors, f"{path} final_grad_norms", final_grad_norms)


def validate_grad_norm_summary(errors: list[str], location: str, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{location} must be a GradNormSummary object")
        return
    expected_fields = {"global_l2", "max_l2", "mean_l2"}
    observed_fields = set(value)
    if observed_fields != expected_fields:
        errors.append(
            f"{location} GradNormSummary fields must be global_l2, max_l2, mean_l2"
        )
        return
    for field in ["global_l2", "max_l2", "mean_l2"]:
        if not finite_number(value.get(field)) or float(value[field]) < 0.0:
            errors.append(f"{location} {field} must be finite and non-negative")


def validate_grad_log(
    errors: list[str],
    path: Path,
    seed: int,
    expected_grad_norms: Any | None,
) -> None:
    records = load_jsonl_objects(errors, path, "grad log")
    if records is None:
        return
    if len(records) != S7_OPTIMIZER_STEPS:
        errors.append(f"{path} grad log must contain {S7_OPTIMIZER_STEPS} completed-run JSONL records")
    expected_by_step: dict[int, Any] = {}
    if isinstance(expected_grad_norms, list):
        for entry in expected_grad_norms:
            if isinstance(entry, list) and len(entry) == 2 and is_positive_int(entry[0]):
                expected_by_step[int(entry[0])] = entry[1]
    previous_step: int | None = None
    for index, record in enumerate(records, start=1):
        location = f"{path}:{index} grad log record"
        schema = record.get("schema")
        if schema != "s7_grad_log.v1":
            errors.append(f"{location} schema must be s7_grad_log.v1, observed {schema!r}")
        require_equal(errors, path, record, "seed", seed)
        train_step = record.get("train_step")
        if not is_positive_int(train_step):
            errors.append(f"{location} train_step must be a positive integer")
        elif previous_step is not None and int(train_step) <= previous_step:
            errors.append(f"{path} grad log train_step values must be strictly increasing")
        if is_positive_int(train_step):
            previous_step = int(train_step)
            expected_step = index
            if int(train_step) != expected_step:
                errors.append(f"{location} train_step must be {expected_step}")
        if "grad_norms" not in record:
            errors.append(f"{location} missing grad_norms")
        else:
            validate_grad_norm_summary(errors, f"{location} grad_norms", record["grad_norms"])
            if is_positive_int(train_step) and expected_by_step:
                expected_summary = expected_by_step.get(int(train_step))
                if expected_summary is not None and record["grad_norms"] != expected_summary:
                    errors.append(f"{location} grad_norms must match run-log grad_norms")


def validate_router_step_telemetry_log(
    errors: list[str], path: Path, topology: str, seed: int
) -> None:
    records = load_jsonl_objects(errors, path, "router-step telemetry")
    if records is None:
        return
    if topology == "MoeTinyDenseMatched" and records:
        errors.append(f"{path} dense router-step telemetry must be empty because dense runs have no router")
        return
    if not records:
        if topology == "MoeTiny":
            errors.append(f"{path} router-step telemetry must contain at least one JSONL record")
        return
    layers_by_step: dict[int, set[int]] = {}
    for index, record in enumerate(records, start=1):
        location = f"{path}:{index} router-step telemetry"
        telemetry = unwrap_router_step_telemetry_record(errors, location, record)
        if telemetry is None:
            continue
        validate_router_step_telemetry(errors, location, telemetry, seed)
        train_step = telemetry.get("train_step")
        layer_id = telemetry.get("layer_id")
        if is_non_negative_int(train_step) and is_layer_id(layer_id):
            layers_by_step.setdefault(int(train_step), set()).add(int(layer_id))
    if topology == "MoeTiny":
        for train_step, layers in sorted(layers_by_step.items()):
            if layers != set(range(S7_N_BLOCKS)):
                observed = ",".join(str(layer) for layer in sorted(layers))
                errors.append(
                    f"{path} router-step telemetry must cover layers 0..3 for train_step {train_step}; observed {observed}"
                )


def load_jsonl_objects(errors: list[str], path: Path, label: str) -> list[dict[str, Any]] | None:
    if not path.is_file():
        errors.append(f"missing {label}: {path}")
        return None
    records: list[dict[str, Any]] = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            errors.append(f"{path}:{line_no} {label} JSONL record must not be blank")
            continue
        try:
            payload = json.loads(line, object_pairs_hook=reject_duplicate_keys)
        except DuplicateKeyError as error:
            errors.append(f"{path}:{line_no} {label} has duplicate JSON key: {error}")
            continue
        except json.JSONDecodeError as error:
            errors.append(f"{path}:{line_no} {label} is not valid JSON: {error}")
            continue
        if not isinstance(payload, dict):
            errors.append(f"{path}:{line_no} {label} JSONL record must be an object")
            continue
        records.append(payload)
    return records


def unwrap_router_step_telemetry_record(
    errors: list[str], location: str, record: dict[str, Any]
) -> dict[str, Any] | None:
    if "telemetry_canonical_json" not in record:
        return record
    event_name = record.get("event_name")
    if event_name != "s7.router.step":
        errors.append(f"{location} event_name must be s7.router.step, observed {event_name!r}")
    payload_text = record.get("telemetry_canonical_json")
    if not isinstance(payload_text, str):
        errors.append(f"{location} telemetry_canonical_json must be a JSON string")
        return None
    try:
        payload = json.loads(payload_text, object_pairs_hook=reject_duplicate_keys)
    except DuplicateKeyError as error:
        errors.append(f"{location} telemetry_canonical_json has duplicate JSON key: {error}")
        return None
    except json.JSONDecodeError as error:
        errors.append(f"{location} telemetry_canonical_json is not valid JSON: {error}")
        return None
    if not isinstance(payload, dict):
        errors.append(f"{location} telemetry_canonical_json must decode to an object")
        return None
    flat_hash = record.get("telemetry_self_hash")
    payload_hash = payload.get("telemetry_self_hash")
    if flat_hash is not None and flat_hash != payload_hash:
        errors.append(f"{location} flat telemetry_self_hash must match telemetry_canonical_json")
    return payload


def validate_router_step_telemetry(
    errors: list[str], location: str, data: dict[str, Any], seed: int
) -> None:
    observed_fields = set(data)
    if observed_fields != ROUTER_STEP_TELEMETRY_FIELDS:
        missing = ", ".join(sorted(ROUTER_STEP_TELEMETRY_FIELDS - observed_fields))
        extra = ", ".join(sorted(observed_fields - ROUTER_STEP_TELEMETRY_FIELDS))
        errors.append(f"{location} RouterStepTelemetry fields mismatch; missing [{missing}] extra [{extra}]")
        return
    if data.get("schema_version") != version_dict():
        errors.append(f"{location} schema_version must be {version_dict()!r}")
    if data.get("seed") != seed:
        errors.append(f"{location} seed must be {seed!r}, observed {data.get('seed')!r}")
    train_step = data.get("train_step")
    if not is_non_negative_int(train_step):
        errors.append(f"{location} train_step must be a non-negative integer")
    layer_id = data.get("layer_id")
    if not is_layer_id(layer_id):
        errors.append(f"{location} layer_id must be 0..3")
    tokens = data.get("tokens_per_expert")
    if not isinstance(tokens, list) or len(tokens) != S7_N_EXPERTS or not all(is_non_negative_int(item) for item in tokens):
        errors.append(f"{location} tokens_per_expert must contain 4 non-negative integer counts")
        token_count_sum: int | None = None
    else:
        token_count_sum = sum(int(item) for item in tokens)
        if token_count_sum <= 0:
            errors.append(f"{location} tokens_per_expert sum must be > 0")
    entropy = data.get("expert_usage_entropy_bits")
    if not finite_number(entropy) or float(entropy) < 0.0:
        errors.append(f"{location} expert_usage_entropy_bits must be finite and non-negative")
    elif isinstance(tokens, list) and len(tokens) > 0 and float(entropy) > math.log2(len(tokens)) + FLOAT_TOL:
        errors.append(f"{location} expert_usage_entropy_bits must be <= log2(n_experts)")
    same_expert_rate = data.get("same_expert_rate")
    if not unit_interval(same_expert_rate):
        errors.append(f"{location} same_expert_rate must be finite and in [0, 1]")
    validate_confidence_dist(errors, location, data.get("router_confidence_distribution"))
    bank_switches = data.get("bank_switches_per_token")
    if not finite_number(bank_switches) or float(bank_switches) < 0.0 or float(bank_switches) > float(S7_N_BLOCKS):
        errors.append(f"{location} bank_switches_per_token must be finite and in [0, n_blocks]")
    verify_domain_self_hash(errors, location, data, "telemetry_self_hash", ROUTER_STEP_TELEMETRY_DOMAIN)


def validate_confidence_dist(errors: list[str], location: str, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{location} router_confidence_distribution must be an object")
        return
    expected_fields = {"mean", "p10", "p50", "p90"}
    if set(value) != expected_fields:
        errors.append(f"{location} router_confidence_distribution fields must be mean, p10, p50, p90")
        return
    for field in ["mean", "p10", "p50", "p90"]:
        if not unit_interval(value.get(field)):
            errors.append(f"{location} router_confidence_distribution.{field} must be finite and in [0, 1]")
    if all(unit_interval(value.get(field)) for field in ["p10", "p50", "p90"]):
        if not float(value["p10"]) <= float(value["p50"]) <= float(value["p90"]):
            errors.append(f"{location} router_confidence_distribution p10 <= p50 <= p90 invariant failed")


def validate_score(errors: list[str], path: Path, topology: str, seed: int) -> None:
    data = load_json(errors, path, "s7_score.v1")
    if data is None:
        return
    require_equal(errors, path, data, "seed", seed)
    require_equal(errors, path, data, "topology", topology)
    require_hash(errors, path, data, "checkpoint_sha")
    require_hash(errors, path, data, "corpus_val_sha")
    require_hash(errors, path, data, "score_self_hash")
    require_equal(errors, path, data, "chunk_size", 256)
    token_count = data.get("token_count")
    log2_sum = data.get("log2_sum")
    bpc = data.get("bpc")
    if not is_positive_int(token_count):
        errors.append(f"{path} token_count must be a positive integer")
    if not finite_number(log2_sum):
        errors.append(f"{path} log2_sum must be finite")
    if not finite_number(bpc):
        errors.append(f"{path} bpc must be finite")
    if is_positive_int(token_count) and finite_number(log2_sum) and finite_number(bpc):
        expected = float(log2_sum) / int(token_count)
        if not math.isclose(float(bpc), expected, rel_tol=FLOAT_TOL, abs_tol=FLOAT_TOL):
            errors.append(f"{path} bpc must equal log2_sum / token_count")
    verify_domain_self_hash(errors, str(path), data, "score_self_hash", SCORE_DOMAIN)


def validate_switch_stats(errors: list[str], path: Path, seed: int) -> None:
    data = load_json(errors, path, "s7_switch_stats.v1")
    if data is None:
        return
    require_equal(errors, path, data, "seed", seed)
    require_non_empty_string(errors, path, data, "artifact_path")
    require_equal(errors, path, data, "aggregation_rule", "SUM")
    require_hash(errors, path, data, "bundle_self_hash")
    verify_domain_self_hash(errors, str(path), data, "bundle_self_hash", SWITCH_STATS_DOMAIN)
    for field in [
        "temporal_switch_digest",
        "clip_saturation_digest",
        "expert_payload_digest",
        "expert_slot_affinity",
    ]:
        value = data.get(field)
        if not isinstance(value, list) or len(value) != 4:
            errors.append(f"{path} {field} must contain 4 layer entries")
            continue
        for layer_id, entry in enumerate(value):
            if not isinstance(entry, dict):
                errors.append(f"{path} {field}[{layer_id}] must be an object")
                continue
            require_equal(errors, path, entry, "layer_id", layer_id)
            if field == "temporal_switch_digest":
                validate_temporal_switch_digest(errors, path, entry, layer_id)
            elif field == "clip_saturation_digest":
                validate_clip_saturation_digest(errors, path, entry, layer_id)
            elif field == "expert_payload_digest":
                validate_expert_payload_digest(errors, path, entry, layer_id)
            elif field == "expert_slot_affinity":
                validate_expert_slot_affinity(errors, path, entry, layer_id)


def validate_temporal_switch_digest(
    errors: list[str], path: Path, entry: dict[str, Any], layer_id: int
) -> None:
    require_equal(errors, path, entry, "n_experts", 4)
    require_q8_8(errors, path, entry, "same_expert_rate_q8_8")
    require_hash(errors, path, entry, "digest_self_hash")
    verify_domain_self_hash(
        errors,
        f"{path} temporal_switch_digest[{layer_id}]",
        entry,
        "digest_self_hash",
        TEMPORAL_SWITCH_DIGEST_DOMAIN,
    )
    transition_mass = entry.get("transition_mass")
    if not isinstance(transition_mass, list) or not transition_mass:
        errors.append(f"{path} temporal_switch_digest[{layer_id}] transition_mass must be non-empty")
        return
    for index, transition in enumerate(transition_mass):
        if not isinstance(transition, dict):
            errors.append(f"{path} temporal_switch_digest[{layer_id}].transition_mass[{index}] must be an object")
            continue
        for field in ["from_expert", "to_expert"]:
            if not is_expert_id(transition.get(field)):
                errors.append(
                    f"{path} temporal_switch_digest[{layer_id}].transition_mass[{index}] {field} must be 0..3"
                )
        if not is_q8_8(transition.get("mass_q8_8")):
            errors.append(
                f"{path} temporal_switch_digest[{layer_id}].transition_mass[{index}] mass_q8_8 must be 0..256"
            )


def validate_clip_saturation_digest(
    errors: list[str], path: Path, entry: dict[str, Any], layer_id: int
) -> None:
    require_q8_8(errors, path, entry, "saturation_rate_q8_8")
    if not finite_number(entry.get("clip_bound_observed")) or float(entry["clip_bound_observed"]) <= 0.0:
        errors.append(f"{path} clip_saturation_digest[{layer_id}] clip_bound_observed must be finite and > 0")
    require_hash(errors, path, entry, "digest_self_hash")
    verify_domain_self_hash(
        errors,
        f"{path} clip_saturation_digest[{layer_id}]",
        entry,
        "digest_self_hash",
        CLIP_SATURATION_DIGEST_DOMAIN,
    )


def validate_expert_payload_digest(
    errors: list[str], path: Path, entry: dict[str, Any], layer_id: int
) -> None:
    require_non_empty_string(errors, path, entry, "artifact_path")
    require_hash(errors, path, entry, "digest_self_hash")
    verify_domain_self_hash(
        errors,
        f"{path} expert_payload_digest[{layer_id}]",
        entry,
        "digest_self_hash",
        EXPERT_PAYLOAD_DIGEST_DOMAIN,
    )
    entries = entry.get("entries")
    if not isinstance(entries, list) or len(entries) != 4:
        errors.append(f"{path} expert_payload_digest[{layer_id}] entries must cover 4 experts")
        return
    observed: set[int] = set()
    for index, payload in enumerate(entries):
        if not isinstance(payload, dict):
            errors.append(f"{path} expert_payload_digest[{layer_id}].entries[{index}] must be an object")
            continue
        expert_id = payload.get("expert_id")
        if not is_expert_id(expert_id):
            errors.append(f"{path} expert_payload_digest[{layer_id}].entries[{index}] expert_id must be 0..3")
        else:
            observed.add(int(expert_id))
        if not is_positive_int(payload.get("byte_count")):
            errors.append(f"{path} expert_payload_digest[{layer_id}].entries[{index}] byte_count must be > 0")
        if "weight_quant" not in payload:
            errors.append(f"{path} expert_payload_digest[{layer_id}].entries[{index}] missing weight_quant")
    if observed != {0, 1, 2, 3}:
        errors.append(f"{path} expert_payload_digest[{layer_id}] entries must exhaust experts 0..3")


def validate_expert_slot_affinity(
    errors: list[str], path: Path, entry: dict[str, Any], layer_id: int
) -> None:
    require_hash(errors, path, entry, "affinity_self_hash")
    verify_domain_self_hash(
        errors,
        f"{path} expert_slot_affinity[{layer_id}]",
        entry,
        "affinity_self_hash",
        EXPERT_SLOT_AFFINITY_DOMAIN,
    )
    affinities = entry.get("affinities")
    if not isinstance(affinities, list):
        errors.append(f"{path} expert_slot_affinity[{layer_id}] affinities must be a list")
        return
    for index, affinity in enumerate(affinities):
        if not isinstance(affinity, dict):
            errors.append(f"{path} expert_slot_affinity[{layer_id}].affinities[{index}] must be an object")
            continue
        pair = affinity.get("pair")
        if not isinstance(pair, dict) or not is_expert_id(pair.get("lo")) or not is_expert_id(pair.get("hi")):
            errors.append(f"{path} expert_slot_affinity[{layer_id}].affinities[{index}] pair must name lo/hi experts 0..3")
        if not is_q8_8(affinity.get("affinity_score")):
            errors.append(f"{path} expert_slot_affinity[{layer_id}].affinities[{index}] affinity_score must be 0..256")


def validate_sweep(errors: list[str], path: Path) -> None:
    data = load_json(errors, path, "s7_router_collapse_sweep.v1")
    if data is None:
        return
    require_equal(errors, path, data, "seed", 0)
    require_hash(errors, path, data, "base_checkpoint_sha")
    require_equal(errors, path, data, "producer_kind", PRODUCTION_SWEEP_PRODUCER_KIND)
    require_equal(errors, path, data, "grid", D11_GRID)
    require_equal(errors, path, data, "production_lambda", 0.05)
    require_equal(errors, path, data, "collapse_threshold", 1.0)
    require_equal(errors, path, data, "guardrail_verdict", "Pass")
    require_hash(errors, path, data, "sweep_self_hash")
    records = data.get("records")
    if not isinstance(records, list) or len(records) != len(D11_GRID):
        errors.append(f"{path} records length must equal D11 grid length")
    else:
        for index, record in enumerate(records):
            validate_sweep_record(errors, path, record, index)
    verify_domain_self_hash(errors, str(path), data, "sweep_self_hash", ROUTER_COLLAPSE_SWEEP_DOMAIN)


def validate_sweep_record(errors: list[str], path: Path, record: Any, index: int) -> None:
    if not isinstance(record, dict):
        errors.append(f"{path} records[{index}] must be an object")
        return
    location = f"{path} records[{index}]"
    require_equal(errors, path, record, "schema_version", version_dict())
    require_equal(errors, path, record, "seed", 0)
    require_equal(errors, path, record, "lambda_switch", D11_GRID[index])
    base_train_step = record.get("base_train_step")
    train_step = record.get("train_step")
    if not is_non_negative_int(base_train_step):
        errors.append(f"{location} base_train_step must be a non-negative integer")
    if not is_non_negative_int(train_step):
        errors.append(f"{location} train_step must be a non-negative integer")
    if is_non_negative_int(base_train_step) and is_non_negative_int(train_step):
        expected_train_step = int(base_train_step) + 1000
        if train_step != expected_train_step:
            errors.append(f"{location} train_step must equal base_train_step + 1000")
    completion = record.get("completion")
    completion_kind = completion.get("kind") if isinstance(completion, dict) else None
    if completion_kind not in {"completed", "diverged_at"}:
        errors.append(f"{location} completion.kind must be completed or diverged_at")
    elif completion_kind == "diverged_at" and not is_positive_int(completion.get("step")):
        errors.append(f"{location} divergent completion.step must be a positive integer")
    if completion_kind == "completed" and completion != {"kind": "completed"}:
        errors.append(f"{location} completed completion must contain only kind=completed")
    if completion_kind == "completed" and not finite_number(record.get("bpc_eval_subset")):
        errors.append(f"{location} bpc_eval_subset must be finite for completed records")
    if completion_kind == "diverged_at" and record.get("bpc_eval_subset") is not None:
        errors.append(f"{location} bpc_eval_subset must be null for divergent records")
    if not finite_number(record.get("expert_usage_entropy_bits_mean")):
        errors.append(f"{location} expert_usage_entropy_bits_mean must be finite")
    elif float(record["expert_usage_entropy_bits_mean"]) < 0.0:
        errors.append(f"{location} expert_usage_entropy_bits_mean must be non-negative")
    quality_delta = record.get("quality_delta_per_lambda_switch")
    if quality_delta is not None and not finite_number(quality_delta):
        errors.append(f"{location} quality_delta_per_lambda_switch must be finite or null")
    require_hash(errors, path, record, "sweep_self_hash")
    verify_domain_self_hash(errors, location, record, "sweep_self_hash", LAMBDA_SWITCH_RECORD_DOMAIN)


def validate_dense_vs_moe(errors: list[str], path: Path) -> None:
    data = load_json(errors, path, "s7_dense_vs_moe.v1")
    if data is None:
        return
    for field in ["moe_topology_hash", "dense_matched_topology_hash", "comparison_self_hash"]:
        require_hash(errors, path, data, field)
    matched_bytes_pin = data.get("matched_bytes_pin")
    tolerance_bytes: int | None = None
    if not isinstance(matched_bytes_pin, dict):
        errors.append(f"{path} matched_bytes_pin must be an object")
    else:
        if not is_semver(matched_bytes_pin.get("formula_version")):
            errors.append(f"{path} matched_bytes_pin.formula_version must be a SemVer object")
        if not non_empty_string(matched_bytes_pin.get("bias_policy")):
            errors.append(f"{path} matched_bytes_pin.bias_policy must be a non-empty string")
        require_hash(errors, path, matched_bytes_pin, "matched_bytes_self_hash")
        verify_domain_self_hash(
            errors,
            f"{path} matched_bytes_pin",
            matched_bytes_pin,
            "matched_bytes_self_hash",
            MATCHED_BYTES_PIN_DOMAIN,
        )
        for field in [
            "d_ff_dense_resolved",
            "b_experts_total",
            "b_router_overhead_total",
            "b_dense_ffn_total",
            "b_deployed_total_moe",
            "b_deployed_total_dense",
            "tolerance_bytes",
        ]:
            if not is_non_negative_int(matched_bytes_pin.get(field)):
                errors.append(f"{path} matched_bytes_pin.{field} must be a non-negative integer")
        if is_non_negative_int(matched_bytes_pin.get("tolerance_bytes")):
            tolerance_bytes = int(matched_bytes_pin["tolerance_bytes"])
    per_seed = data.get("per_seed")
    valid_per_seed: list[dict[str, Any]] = []
    if not isinstance(per_seed, list) or len(per_seed) != 5:
        errors.append(f"{path} per_seed must contain 5 entries")
    else:
        observed = {entry.get("seed") for entry in per_seed if isinstance(entry, dict)}
        if observed != set(SEEDS):
            errors.append(f"{path} per_seed must cover seeds 0..4")
        for entry in per_seed:
            if not isinstance(entry, dict):
                errors.append(f"{path} per_seed entries must be objects")
                continue
            for field in ["val_bpc_moe", "val_bpc_dense", "delta"]:
                if not finite_number(entry.get(field)):
                    errors.append(f"{path} seed {entry.get('seed')} {field} must be finite")
            if finite_number(entry.get("val_bpc_moe")) and finite_number(entry.get("val_bpc_dense")) and finite_number(entry.get("delta")):
                expected_delta = float(entry["val_bpc_dense"]) - float(entry["val_bpc_moe"])
                if not math.isclose(float(entry["delta"]), expected_delta, rel_tol=FLOAT_TOL, abs_tol=FLOAT_TOL):
                    errors.append(f"{path} seed {entry.get('seed')} delta must be dense - moe")
                expected_parity = derive_parity_verdict(
                    float(entry["val_bpc_moe"]), float(entry["val_bpc_dense"])
                )
                if entry.get("parity_verdict") != expected_parity:
                    errors.append(
                        f"{path} seed {entry.get('seed')} parity_verdict must be {expected_parity}"
                    )
                valid_per_seed.append(entry)
    for field in [
        "median_val_bpc_moe",
        "median_val_bpc_dense",
        "deployed_bytes_total_moe",
        "deployed_bytes_total_dense",
        "bytes_diff",
    ]:
        if not finite_number(data.get(field)) and not isinstance(data.get(field), int):
            errors.append(f"{path} {field} must be numeric")
    if all(is_non_negative_int(data.get(field)) for field in ["deployed_bytes_total_moe", "deployed_bytes_total_dense"]):
        expected_bytes_diff = int(data["deployed_bytes_total_dense"]) - int(data["deployed_bytes_total_moe"])
        if data.get("bytes_diff") != expected_bytes_diff:
            errors.append(f"{path} bytes_diff must be dense - moe")
        if tolerance_bytes is not None:
            expected_within = abs(expected_bytes_diff) <= tolerance_bytes
            if data.get("bytes_within_tolerance") is not expected_within:
                errors.append(f"{path} bytes_within_tolerance must match |bytes_diff| <= tolerance_bytes")
            if data.get("bytes_within_tolerance") is not True:
                errors.append(f"{path} bytes_within_tolerance must be true for bd-2v9r closure")
            if len(valid_per_seed) == 5:
                expected_aggregate = derive_aggregate_parity_verdict(valid_per_seed, expected_within)
                if data.get("aggregate_parity_verdict") != expected_aggregate:
                    errors.append(f"{path} aggregate_parity_verdict must match derived {expected_aggregate}")
            if finite_number(data.get("median_val_bpc_moe")) and finite_number(data.get("median_val_bpc_dense")):
                expected_pareto = derive_pareto_verdict(
                    float(data["median_val_bpc_moe"]),
                    float(data["median_val_bpc_dense"]),
                    int(data["deployed_bytes_total_moe"]),
                    int(data["deployed_bytes_total_dense"]),
                    tolerance_bytes,
                )
                if data.get("pareto_verdict") != expected_pareto:
                    errors.append(f"{path} pareto_verdict must match derived {expected_pareto}")
    validate_switch_stats_summary(errors, path, data.get("switch_stats_summary"))
    validate_sweep_summary(errors, path, data.get("sweep_summary"))
    verify_domain_self_hash(errors, str(path), data, "comparison_self_hash", DENSE_VS_MOE_DOMAIN)


def validate_frontier(errors: list[str], path: Path) -> None:
    data = load_json(errors, path, "s7_frontier.v1")
    if data is None:
        return
    require_hash(errors, path, data, "frontier_self_hash")
    verify_domain_self_hash(errors, str(path), data, "frontier_self_hash", FRONTIER_DOMAIN)
    if data.get("pareto_verdict") not in PARETO_VERDICTS:
        errors.append(f"{path} pareto_verdict must be a known ParetoVerdict")
    points = data.get("points")
    if not isinstance(points, list) or len(points) != 2:
        errors.append(f"{path} points must contain one MoE and one dense point")
    else:
        observed = {point.get("topology") for point in points if isinstance(point, dict)}
        if observed != set(TOPOLOGIES):
            errors.append(f"{path} points must cover {TOPOLOGIES}")
        for point in points:
            if not isinstance(point, dict):
                errors.append(f"{path} frontier points must be objects")
                continue
            validate_frontier_point(errors, path, point)


def validate_frontier_point(errors: list[str], path: Path, point: dict[str, Any]) -> None:
    topology = point.get("topology")
    require_hash(errors, path, point, "checkpoint_sha")
    quality = point.get("quality")
    if not isinstance(quality, dict):
        errors.append(f"{path} frontier point {topology} quality must be an object")
    else:
        if not finite_number(quality.get("median_val_bpc")):
            errors.append(f"{path} frontier point {topology} quality.median_val_bpc must be finite")
        values = quality.get("per_seed_val_bpc")
        if not isinstance(values, list) or len(values) != 5 or not all(finite_number(value) for value in values):
            errors.append(f"{path} frontier point {topology} quality.per_seed_val_bpc must contain 5 finite values")
    conformance = point.get("conformance")
    if not isinstance(conformance, dict) or not conformance:
        errors.append(f"{path} frontier point {topology} conformance must be a non-empty object")
    projected_fit = point.get("projected_fit")
    if not isinstance(projected_fit, dict):
        errors.append(f"{path} frontier point {topology} projected_fit must be an object")
    else:
        if not is_positive_int(projected_fit.get("deployed_bytes_total")):
            errors.append(f"{path} frontier point {topology} projected_fit.deployed_bytes_total must be > 0")
        per_block = projected_fit.get("deployed_bytes_per_block")
        if not isinstance(per_block, list) or len(per_block) != 4 or not all(is_positive_int(value) for value in per_block):
            errors.append(f"{path} frontier point {topology} projected_fit.deployed_bytes_per_block must contain 4 positive integers")
    if "schedule_cost" not in point:
        errors.append(f"{path} frontier point {topology} missing schedule_cost")


def validate_burn_grad(errors: list[str], path: Path) -> None:
    data = load_json(errors, path, "s7_burn_grad_smoke.v1")
    if data is None:
        return
    require_equal(errors, path, data, "fixture_seed", 0xFEED)
    value = data.get("burn_adapter_version")
    if not isinstance(value, str) or not value:
        errors.append(f"{path} burn_adapter_version must be a non-empty string")
    for field in [
        "grad_up_weight_sum_abs",
        "grad_down_weight_sum_abs",
    ]:
        value = data.get(field)
        if not finite_number(value) or float(value) <= 0.0:
            errors.append(f"{path} {field} must be finite and > 0")
    for field in [
        "grad_up_bias_sum_abs",
        "grad_down_bias_sum_abs",
        "grad_activation_clip_threshold_sum_abs",
    ]:
        if field in data:
            errors.append(
                f"{path} {field} is unsupported because ExpertBlockQat bias and learned activation-range parameters are rejected"
            )
    require_equal(errors, path, data, "supported_clipped_activation_count", 3)
    require_equal(errors, path, data, "learned_activation_range_unsupported", True)
    require_equal(errors, path, data, "projection_biases_unsupported", True)
    require_equal(errors, path, data, "glu_construction_rejected", True)
    require_equal(errors, path, data, "replay_byte_identical", True)
    require_hash(errors, path, data, "fixture_input_sha")
    require_hash(errors, path, data, "smoke_self_hash")
    verify_domain_self_hash(errors, str(path), data, "smoke_self_hash", BURN_GRAD_SMOKE_DOMAIN)


def validate_oracle(errors: list[str], path: Path) -> None:
    data = load_json(errors, path, "s7_oracle_routed.v1")
    if data is None:
        return
    require_equal(errors, path, data, "seed", 0)
    require_equal(errors, path, data, "topology", "MoeTiny")
    for field in [
        "fixture_prompt_sha",
        "train_logits_sha",
        "bundle_logits_sha",
        "artifact_logits_sha",
        "frozen_teacher_checkpoint_sha",
        "oracle_self_hash",
    ]:
        require_hash(errors, path, data, field)
    verify_domain_self_hash(errors, str(path), data, "oracle_self_hash", ORACLE_ROUTED_DOMAIN)
    require_equal(errors, path, data, "weight_quant_resolution", "QuantSpec::weight_quant")
    tolerance = data.get("s3_tolerance")
    for field in [
        "pairwise_max_abs_diff_train_bundle",
        "pairwise_max_abs_diff_bundle_artifact",
        "pairwise_max_abs_diff_train_artifact",
    ]:
        value = data.get(field)
        if not finite_number(value):
            errors.append(f"{path} {field} must be finite")
        elif finite_number(tolerance) and float(value) > float(tolerance):
            errors.append(f"{path} {field} exceeds s3_tolerance")
    coverage = data.get("route_coverage")
    if not isinstance(coverage, dict) or not all(
        coverage.get(field) is True
        for field in [
            "cross_layer_route_difference",
            "consecutive_token_route_change",
            "consecutive_token_route_same",
        ]
    ):
        errors.append(f"{path} route_coverage must prove all routed fixture axes")


def validate_emulator(errors: list[str], path: Path, topology: str) -> None:
    data = load_json(errors, path, "s7_emulator_one_token.v1")
    if data is None:
        return
    require_equal(errors, path, data, "seed", 0)
    require_equal(errors, path, data, "topology", topology)
    for field in [
        "encoded_rom_sha",
        "prompt_sha",
        "artifact_oracle_logits_sha",
        "emulator_logits_sha",
        "emulator_self_hash",
    ]:
        require_hash(errors, path, data, field)
    tolerance = data.get("s5_tolerance")
    diff = data.get("pairwise_max_abs_diff")
    if not finite_number(diff):
        errors.append(f"{path} pairwise_max_abs_diff must be finite")
    elif finite_number(tolerance) and float(diff) > float(tolerance):
        errors.append(f"{path} pairwise_max_abs_diff exceeds s5_tolerance")
    if data.get("bank_switch_within_one") is not True:
        errors.append(f"{path} bank_switch_within_one must be true")
    if finite_number(data.get("bank_switch_diff")) and float(data["bank_switch_diff"]) > 1.0:
        errors.append(f"{path} bank_switch_diff must be <= 1")
    verify_domain_self_hash(errors, str(path), data, "emulator_self_hash", EMULATOR_ONE_TOKEN_DOMAIN)


def require_equal(
    errors: list[str], path: Path, data: dict[str, Any], field: str, expected: Any
) -> None:
    if data.get(field) != expected:
        errors.append(f"{path} {field} must be {expected!r}, observed {data.get(field)!r}")


def require_hash(errors: list[str], path: Path, data: dict[str, Any], field: str) -> None:
    if not is_hash(data.get(field)):
        errors.append(f"{path} {field} must be a non-null sha256 hash")


def require_q8_8(errors: list[str], path: Path, data: dict[str, Any], field: str) -> None:
    if not is_q8_8(data.get(field)):
        errors.append(f"{path} {field} must be an integer in 0..256")


def require_non_empty_list(
    errors: list[str], path: Path, data: dict[str, Any], field: str
) -> None:
    value = data.get(field)
    if not isinstance(value, list) or not value:
        errors.append(f"{path} {field} must be a non-empty list")


def require_non_empty_string(
    errors: list[str], path: Path, data: dict[str, Any], field: str
) -> None:
    value = data.get(field)
    if not isinstance(value, str) or not value:
        errors.append(f"{path} {field} must be a non-empty string")


def is_hash(value: Any) -> bool:
    return isinstance(value, str) and bool(HASH_RE.match(value))


def non_empty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value)


def finite_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def unit_interval(value: Any) -> bool:
    return finite_number(value) and 0.0 <= float(value) <= 1.0


def is_positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def is_non_negative_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def is_semver(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and set(value) == {"major", "minor", "patch"}
        and all(is_non_negative_int(value[field]) for field in ["major", "minor", "patch"])
    )


def is_q8_8(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 256


def is_expert_id(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and 0 <= value < S7_N_EXPERTS


def is_layer_id(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and 0 <= value < S7_N_BLOCKS


def derive_parity_verdict(val_bpc_moe: float, val_bpc_dense: float) -> str:
    if val_bpc_moe < val_bpc_dense - S7_PARITY_BPC_MARGIN:
        return "Pass"
    return "Fail"


def derive_aggregate_parity_verdict(
    per_seed: list[dict[str, Any]], bytes_within_tolerance: bool
) -> str:
    if not bytes_within_tolerance:
        return "Fail-bytes"
    if all(entry.get("parity_verdict") == "Pass" for entry in per_seed):
        return "Pass-clean"
    return "Fail-parity"


def derive_pareto_verdict(
    median_val_bpc_moe: float,
    median_val_bpc_dense: float,
    deployed_bytes_total_moe: int,
    deployed_bytes_total_dense: int,
    tolerance_bytes: int,
) -> str:
    bpc_equal = math.isclose(
        median_val_bpc_moe, median_val_bpc_dense, rel_tol=FLOAT_TOL, abs_tol=FLOAT_TOL
    )
    bpc_moe_less = median_val_bpc_moe < median_val_bpc_dense and not bpc_equal
    bpc_dense_less = median_val_bpc_dense < median_val_bpc_moe and not bpc_equal
    bpc_le_moe = bpc_moe_less or bpc_equal
    bpc_le_dense = bpc_dense_less or bpc_equal
    by_le_moe = deployed_bytes_total_moe <= deployed_bytes_total_dense
    by_le_dense = deployed_bytes_total_dense <= deployed_bytes_total_moe
    if bpc_le_moe and by_le_moe and (bpc_moe_less or deployed_bytes_total_moe < deployed_bytes_total_dense):
        return "MoE-dominates"
    if bpc_le_dense and by_le_dense and (
        bpc_dense_less or deployed_bytes_total_dense < deployed_bytes_total_moe
    ):
        return "dense-dominates"
    if bpc_equal and deployed_bytes_total_moe == deployed_bytes_total_dense:
        return "Tied"
    bytes_equivalent = abs(deployed_bytes_total_moe - deployed_bytes_total_dense) <= tolerance_bytes
    if bytes_equivalent and bpc_moe_less:
        return "MoE-wins-under-byte-equivalence"
    if bytes_equivalent and bpc_dense_less:
        return "Dense-wins-under-byte-equivalence"
    return "Incomparable"


def validate_switch_stats_summary(errors: list[str], path: Path, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{path} switch_stats_summary must be an object")
        return
    rates = value.get("same_expert_rate_per_layer_q8_8")
    if not isinstance(rates, list) or len(rates) != 4 or not all(is_q8_8(rate) for rate in rates):
        errors.append(f"{path} switch_stats_summary.same_expert_rate_per_layer_q8_8 must contain 4 q8.8 values")
    for field in ["expert_usage_entropy_bits_mean", "bank_switches_per_token_mean"]:
        if not finite_number(value.get(field)) or float(value[field]) < 0.0:
            errors.append(f"{path} switch_stats_summary.{field} must be finite and non-negative")


def validate_sweep_summary(errors: list[str], path: Path, value: Any) -> None:
    if not isinstance(value, dict):
        errors.append(f"{path} sweep_summary must be an object")
        return
    for field in ["bpc_at_lambda", "entropy_at_lambda"]:
        table = value.get(field)
        if not isinstance(table, dict) or not table:
            errors.append(f"{path} sweep_summary.{field} must be a non-empty object")
        elif not all(finite_number(item) for item in table.values()):
            errors.append(f"{path} sweep_summary.{field} values must be finite")
    if "guardrail_verdict" not in value:
        errors.append(f"{path} sweep_summary missing guardrail_verdict")


if __name__ == "__main__":
    sys.exit(main())
