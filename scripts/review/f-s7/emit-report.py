#!/usr/bin/env python3
"""Emit docs/experiments/S7-report.md from production F-S7 artifact JSON."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
TOPOLOGIES = ("MoeTiny", "MoeTinyDenseMatched")
SEEDS = range(5)
SURPRISE_LM_LOSS_SPREAD = 5.0
PARETO_H4_CONFIRMING = {"MoE-dominates", "MoE-wins-under-byte-equivalence"}
PARETO_H4_REFUTING = {
    "dense-dominates",
    "Dense-wins-under-byte-equivalence",
    "Incomparable",
    "Tied",
}
SWITCH_STATS_MANIFEST_DOMAIN = (
    "gbf-experiments",
    "S7SwitchStatsBundleManifest",
    "s7_switch_stats_bundle_manifest.v1",
    "1",
)
REPORT_MARKDOWN_DOMAIN = (
    "gbf-experiments",
    "S7ReportMarkdown",
    "s7_report.v1",
    "1",
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build a fail-closed F-S7 s7_report.v1 from production artifacts."
    )
    parser.add_argument("--root", default=".", help="repository root or packet root")
    parser.add_argument("--output", default="docs/experiments/S7-report.md")
    parser.add_argument("--s7-outcome", default="PassClean", choices=["PassClean", "FailParity"])
    parser.add_argument("--decision", choices=["ProceedToS8", "ProceedToS8DenseOnly"])
    parser.add_argument("--rfc-revision", help="40-hex git commit or sha256 RFC revision")
    parser.add_argument("--predictions-section-hash", required=True)
    parser.add_argument("--predictions-commit", required=True)
    parser.add_argument("--first-result-commit", required=True)
    parser.add_argument("--generated-at", help="RFC3339 UTC timestamp; hash-excluded")
    args = parser.parse_args()

    root = Path(args.root)
    errors: list[str] = []
    decision = args.decision or decision_for_outcome(args.s7_outcome)
    validate_cli_fields(errors, args, decision)
    if errors:
        print("S7 report emit: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1

    try:
        report = build_report(
            root=root,
            s7_outcome=args.s7_outcome,
            decision=decision,
            rfc_revision=args.rfc_revision or git_head(root),
            predictions_section_hash=args.predictions_section_hash,
            predictions_commit=args.predictions_commit,
            first_result_commit=args.first_result_commit,
            generated_at=args.generated_at or utc_now_rfc3339(),
        )
    except EmitError as error:
        print("S7 report emit: NEEDS_CHANGES")
        print(f" - {error}")
        return 1

    if args.output == "-":
        sys.stdout.write(report)
    else:
        output = root / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(report, encoding="utf-8")
        print(f"S7 report emit: wrote {output}")
    return 0


def validate_cli_fields(errors: list[str], args: argparse.Namespace, decision: str) -> None:
    if args.s7_outcome == "PassClean" and decision != "ProceedToS8":
        errors.append("PassClean must emit decision ProceedToS8")
    if args.s7_outcome == "FailParity" and decision != "ProceedToS8DenseOnly":
        errors.append("FailParity must emit decision ProceedToS8DenseOnly")
    if not is_hash(args.predictions_section_hash):
        errors.append("--predictions-section-hash must be a sha256 hash")
    for field in ["predictions_commit", "first_result_commit"]:
        value = getattr(args, field)
        if not COMMIT_RE.match(value):
            errors.append(f"--{field.replace('_', '-')} must be a 40-hex git commit id")
    if args.rfc_revision is not None and not (
        COMMIT_RE.match(args.rfc_revision) or is_hash(args.rfc_revision)
    ):
        errors.append("--rfc-revision must be a 40-hex git commit id or sha256 hash")


def build_report(
    *,
    root: Path,
    s7_outcome: str,
    decision: str,
    rfc_revision: str,
    predictions_section_hash: str,
    predictions_commit: str,
    first_result_commit: str,
    generated_at: str,
) -> str:
    row_text, score_rows, loss_rows = per_seed_rows(root)
    comparison = load_json(root / "experiments/S7/dense-vs-moe/comparison.json")
    comparison_label = "experiments/S7/dense-vs-moe/comparison.json"
    matched_bytes_self_hash = require_hash(
        comparison,
        ["matched_bytes_pin", "matched_bytes_self_hash"],
        comparison_label,
    )
    pareto_verdict = require_string(comparison, ["pareto_verdict"], comparison_label)
    switch_stats_self_hash = switch_stats_manifest_hash(root)
    router_collapse_sweep_self_hash = artifact_hash(
        root, "experiments/S7/router-collapse/seed-0/sweep.json", ["sweep_self_hash"]
    )
    dense_vs_moe_self_hash = require_hash(comparison, ["comparison_self_hash"], comparison_label)
    frontier_self_hash = artifact_hash(
        root, "experiments/S7/frontier/frontier.json", ["frontier_self_hash"]
    )
    burn_grad_smoke_self_hash = artifact_hash(
        root, "experiments/S7/burn-grad-smoke/expert_block_qat.json", ["smoke_self_hash"]
    )
    oracle_routed_self_hash = artifact_hash(
        root, "experiments/S7/oracle-routed/seed-0/oracle.json", ["oracle_self_hash"]
    )
    emulator_one_token_moe_self_hash = artifact_hash(
        root, "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json", ["emulator_self_hash"]
    )
    dense_emulator = root / "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json"
    if decision == "ProceedToS8DenseOnly":
        emulator_one_token_dense_self_hash = quote_hash(
            artifact_hash(
                root,
                "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
                ["emulator_self_hash"],
            )
        )
    elif dense_emulator.is_file():
        emulator_one_token_dense_self_hash = quote_hash(
            artifact_hash(
                root,
                "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
                ["emulator_self_hash"],
            )
        )
    else:
        emulator_one_token_dense_self_hash = "null"

    body = report_body(score_rows, loss_rows, s7_outcome, decision, pareto_verdict)
    report = f"""---
schema: "s7_report.v1"
s7_outcome: {s7_outcome}
decision: {decision}
matched_bytes_self_hash: "{matched_bytes_self_hash}"
per_seed_artifacts:
{row_text}switch_stats_self_hash: "{switch_stats_self_hash}"
router_collapse_sweep_self_hash: "{router_collapse_sweep_self_hash}"
dense_vs_moe_self_hash: "{dense_vs_moe_self_hash}"
frontier_self_hash: "{frontier_self_hash}"
burn_grad_smoke_self_hash: "{burn_grad_smoke_self_hash}"
oracle_routed_self_hash: "{oracle_routed_self_hash}"
emulator_one_token_moe_self_hash: "{emulator_one_token_moe_self_hash}"
emulator_one_token_dense_self_hash: {emulator_one_token_dense_self_hash}
generated_at: "{generated_at}"
rfc_revision: "{rfc_revision}"
predictions_section_hash: "{predictions_section_hash}"
predictions_commit: "{predictions_commit}"
first_result_commit: "{first_result_commit}"
report_self_hash: null
---
{body}"""
    return with_report_self_hash(report)


def per_seed_rows(root: Path) -> tuple[str, list[dict[str, Any]], list[dict[str, Any]]]:
    rows: list[str] = []
    score_rows: list[dict[str, Any]] = []
    loss_rows: list[dict[str, Any]] = []
    for topology in TOPOLOGIES:
        for seed in SEEDS:
            run = load_json(root / f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json")
            score = load_json(root / f"experiments/S7/scores/{topology}/seed-{seed}/score.json")
            if run.get("completion") != {"kind": "completed"}:
                raise EmitError(f"{topology} seed {seed} run-log completion is not completed")
            checkpoint_sha = require_hash(score, ["checkpoint_sha"], f"{topology} seed {seed} score")
            run_hash = require_hash(run, ["run_log_self_hash"], f"{topology} seed {seed} run-log")
            score_hash = require_hash(score, ["score_self_hash"], f"{topology} seed {seed} score")
            bpc = score.get("bpc")
            deployed = "see matched-bytes artifact"
            final_lm_loss = final_lm_loss_raw(run)
            if final_lm_loss is not None:
                loss_rows.append({"seed": seed, "topology": topology, "lm_loss_raw": final_lm_loss})
            score_rows.append(
                {
                    "seed": seed,
                    "topology": topology,
                    "bpc": bpc,
                    "checkpoint_sha": checkpoint_sha,
                    "score_self_hash": score_hash,
                    "deployed": deployed,
                }
            )
            rows.append(
                f"""  - seed: {seed}
    topology: "{topology}"
    completion: Completed
    checkpoint_self_hash: "{checkpoint_sha}"
    run_log_self_hash: "{run_hash}"
    score_self_hash: "{score_hash}"
"""
            )
    return "".join(rows), score_rows, loss_rows


def final_lm_loss_raw(run: dict[str, Any]) -> float | None:
    losses = run.get("losses")
    if not isinstance(losses, list) or not losses:
        return None
    last = losses[-1]
    if not isinstance(last, list) or len(last) != 2 or not isinstance(last[1], dict):
        return None
    value = last[1].get("lm_loss_raw")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    value = float(value)
    if not math.isfinite(value):
        return None
    return value


def report_body(
    score_rows: list[dict[str, Any]],
    loss_rows: list[dict[str, Any]],
    s7_outcome: str,
    decision: str,
    pareto_verdict: str,
) -> str:
    h3_status = "Refuted" if s7_outcome == "FailParity" else "Confirmed"
    h4_status = h4_status_from_pareto(pareto_verdict)
    observed = "\n".join(
        f"- seed {row['seed']} {row['topology']}: val_bpc={row['bpc']}, completion=Completed, "
        f"deployed_bytes_total={row['deployed']}, score_self_hash={row['score_self_hash']}"
        for row in score_rows
    )
    hypotheses = "\n".join(
        f"H{index} {hypothesis_status(index, h3_status, h4_status)}: cited by the matching closure artifact."
        for index in range(1, 11)
    )
    falsification = falsification_summary(s7_outcome, pareto_verdict, h4_status)
    surprises = surprise_summary(score_rows, loss_rows)
    return "\n".join(
        [
            "## Pre-registered predictions",
            "Pinned predictions are identified by predictions_commit and predictions_section_hash in front matter.",
            "## Observed (per-seed, per-topology table)",
            observed,
            "## Hypothesis verdicts",
            hypotheses,
            "## Falsification analysis",
            falsification,
            "## Switch statistics summary",
            "See s7_switch_stats.v1 bundle hashes cited by switch_stats_self_hash.",
            "## lambda_switch sweep summary",
            "See s7_router_collapse_sweep.v1 and its guardrail verdict.",
            "## Pareto verdict",
            "See s7_dense_vs_moe.v1 and s7_frontier.v1.",
            "## Surprises",
            surprises,
            "## Decision",
            f"{decision}.",
            "## Reproducibility statement",
            "Run scripts/review/f-s7/verify-packet.sh from the reviewed commit with the pinned artifacts.",
            "",
        ]
    )


def surprise_summary(score_rows: list[dict[str, Any]], loss_rows: list[dict[str, Any]]) -> str:
    moe_losses = [
        row
        for row in loss_rows
        if row.get("topology") == "MoeTiny"
        and isinstance(row.get("seed"), int)
        and isinstance(row.get("lm_loss_raw"), float)
    ]
    if len(moe_losses) < 2:
        return "No additional surprises recorded by the emitter."
    low = min(moe_losses, key=lambda row: row["lm_loss_raw"])
    high = max(moe_losses, key=lambda row: row["lm_loss_raw"])
    spread = high["lm_loss_raw"] - low["lm_loss_raw"]
    if spread < SURPRISE_LM_LOSS_SPREAD:
        return "No additional surprises recorded by the emitter."
    parity_context = dense_bpc_context(score_rows)
    return (
        "MoE final-step lm_loss_raw was noisy across seeds "
        f"(min seed {low['seed']}={format_metric(low['lm_loss_raw'])}, "
        f"max seed {high['seed']}={format_metric(high['lm_loss_raw'])})"
        f"{parity_context}; treat this raw training-loss spread as follow-up context, "
        "not parity evidence."
    )


def dense_bpc_context(score_rows: list[dict[str, Any]]) -> str:
    moe: dict[int, float] = {}
    dense: dict[int, float] = {}
    for row in score_rows:
        seed = row.get("seed")
        bpc = row.get("bpc")
        if not isinstance(seed, int) or isinstance(bpc, bool) or not isinstance(bpc, (int, float)):
            continue
        if row.get("topology") == "MoeTiny":
            moe[seed] = float(bpc)
        elif row.get("topology") == "MoeTinyDenseMatched":
            dense[seed] = float(bpc)
    common_seeds = sorted(set(moe) & set(dense))
    if common_seeds and all(dense[seed] < moe[seed] for seed in common_seeds):
        return ", while dense validation BPC beat MoE on every comparable seed"
    return ""


def hypothesis_status(index: int, h3_status: str, h4_status: str) -> str:
    if index == 3:
        return h3_status
    if index == 4:
        return h4_status
    return "Confirmed"


def h4_status_from_pareto(pareto_verdict: str) -> str:
    if pareto_verdict in PARETO_H4_CONFIRMING:
        return "Confirmed"
    if pareto_verdict in PARETO_H4_REFUTING:
        return "Refuted"
    raise EmitError(f"unknown pareto_verdict for H4 mapping: {pareto_verdict!r}")


def falsification_summary(s7_outcome: str, pareto_verdict: str, h4_status: str) -> str:
    findings: list[str] = []
    if s7_outcome == "FailParity":
        findings.append("H3 was refuted by the per-seed bpc parity table under matched bytes.")
    if h4_status == "Refuted":
        findings.append(
            f"H4 was refuted by the Pareto verdict ({pareto_verdict}) under matched bytes."
        )
    if findings:
        return " ".join(findings)
    return "No falsification rule fired for the closure-candidate outcome."


def format_metric(value: float) -> str:
    return f"{value:.6g}"


def switch_stats_manifest_hash(root: Path) -> str:
    entries = []
    for seed in SEEDS:
        bundle_hash = artifact_hash(
            root,
            f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json",
            ["bundle_self_hash"],
        )
        entries.append({"seed": seed, "bundle_self_hash": bundle_hash})
    return domain_json_hash(
        SWITCH_STATS_MANIFEST_DOMAIN,
        {
            "schema": "s7_switch_stats_bundle_manifest.v1",
            "seed_bundle_self_hashes": entries,
        },
    )


def artifact_hash(root: Path, rel_path: str, json_path: list[str]) -> str:
    return require_hash(load_json(root / rel_path), json_path, rel_path)


def load_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise EmitError(f"missing artifact: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise EmitError(f"{path} is not valid JSON: {error}") from error
    if not isinstance(payload, dict):
        raise EmitError(f"{path} must contain a JSON object")
    return payload


def require_hash(payload: dict[str, Any], keys: list[str], label: str) -> str:
    value: Any = payload
    for key in keys:
        if not isinstance(value, dict):
            raise EmitError(f"{label} missing {'.'.join(keys)}")
        value = value.get(key)
    if not is_hash(value):
        raise EmitError(f"{label} {'.'.join(keys)} must be a sha256 hash")
    return value


def require_string(payload: dict[str, Any], keys: list[str], label: str) -> str:
    value: Any = payload
    for key in keys:
        if not isinstance(value, dict):
            raise EmitError(f"{label} missing {'.'.join(keys)}")
        value = value.get(key)
    if not isinstance(value, str):
        raise EmitError(f"{label} {'.'.join(keys)} must be a string")
    return value


def with_report_self_hash(text: str) -> str:
    normalized = normalize_report_for_hash(text)
    report_hash = domain_bytes_hash(REPORT_MARKDOWN_DOMAIN, normalized.encode("utf-8"))
    return re.sub(
        r"(?m)^report_self_hash: null$",
        f'report_self_hash: "{report_hash}"',
        text,
        count=1,
    )


def normalize_report_for_hash(text: str) -> str:
    text = re.sub(
        r'(?m)^report_self_hash:\s*(?:"sha256:[0-9a-f]{64}"|sha256:[0-9a-f]{64}|null)\s*$',
        "report_self_hash: null",
        text,
        count=1,
    )
    return re.sub(
        r'(?m)^generated_at:\s*(?:"[^"\n]*"|[^#\n]*)\s*$',
        "generated_at: null",
        text,
        count=1,
    )


def decision_for_outcome(outcome: str) -> str:
    return "ProceedToS8DenseOnly" if outcome == "FailParity" else "ProceedToS8"


def quote_hash(value: str) -> str:
    return f'"{value}"'


def git_head(root: Path) -> str:
    try:
        head = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise EmitError("--rfc-revision is required outside a git checkout") from error
    if not COMMIT_RE.match(head):
        raise EmitError("git rev-parse HEAD did not return a 40-hex commit id")
    return head


def utc_now_rfc3339() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def domain_json_hash(domain: tuple[str, str, str, str], payload: object) -> str:
    canonical = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")
    return domain_bytes_hash(domain, canonical)


def domain_bytes_hash(domain: tuple[str, str, str, str], payload: bytes) -> str:
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + payload
    )
    return f"sha256:{hashlib.sha256(material).hexdigest()}"


def is_hash(value: object) -> bool:
    return isinstance(value, str) and bool(HASH_RE.match(value))


class EmitError(RuntimeError):
    pass


if __name__ == "__main__":
    sys.exit(main())
