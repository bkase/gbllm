#!/usr/bin/env python3
"""Assemble the F-S7 closure packet from a real production bundle manifest."""

from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any


SCHEMA = "s7_production_bundle_manifest.v1"
TOPOLOGIES = ("MoeTiny", "MoeTinyDenseMatched")
SEEDS = range(5)
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
REQUIRED_RUN_FIELDS = ("run_log", "score", "grad_log", "router_step_telemetry")
REQUIRED_SUPPORT_ARTIFACTS = (
    "router_collapse_sweep",
    "burn_grad_smoke",
    "oracle_routed",
    "emulator_one_token_moe",
)
TOP_LEVEL_FIELDS = {
    "schema",
    "runs",
    "switch_stats",
    "support_artifacts",
    "comparison",
    "frontier",
    "report",
}
SUPPORT_FIELDS = {*REQUIRED_SUPPORT_ARTIFACTS, "emulator_one_token_dense"}
COMPARISON_FIELDS = {"moe_topology_hash", "dense_matched_topology_hash"}
FRONTIER_FIELDS = {
    "moe_conformance",
    "dense_conformance",
    "moe_deployed_bytes_per_block",
    "dense_deployed_bytes_per_block",
    "moe_schedule_cost",
    "dense_schedule_cost",
}
REPORT_FIELDS = {
    "s7_outcome",
    "decision",
    "rfc_revision",
    "predictions_section_hash",
    "predictions_commit",
    "first_result_commit",
    "generated_at",
}


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Materialize all F-S7 production artifacts from an external bundle "
            "manifest, derive dependent packet artifacts, emit the final report, "
            "and run the packet verifier."
        )
    )
    parser.add_argument("--manifest", help="s7_production_bundle_manifest.v1 JSON")
    parser.add_argument("--root", default=".", help="repository/packet root to populate")
    parser.add_argument("--cargo", default="cargo", help="cargo executable")
    parser.add_argument("--verify-mode", choices=["full", "skip-gates"], default="full")
    parser.add_argument("--dry-run", action="store_true", help="print commands without executing")
    parser.add_argument(
        "--run-reviews",
        action="store_true",
        help="run required Gemini/Claude ACPX reviews after report emission and before final verify",
    )
    parser.add_argument(
        "--review-cwd",
        help="ACPX --cwd value used when --run-reviews is set; defaults to --root",
    )
    parser.add_argument("--acpx", default="acpx", help="acpx executable used with --run-reviews")
    parser.add_argument(
        "--review-timeout",
        default="1800",
        help="ACPX timeout in seconds used with --run-reviews",
    )
    parser.add_argument(
        "--reviewer",
        choices=["gemini", "claude", "all"],
        default="all",
        help="reviewer selection passed to run-acpx-reviews.py with --run-reviews",
    )
    parser.add_argument(
        "--gemini-agent",
        help="optional raw Gemini ACP agent command passed to run-acpx-reviews.py",
    )
    parser.add_argument(
        "--claude-agent",
        help="optional raw Claude ACP agent command passed to run-acpx-reviews.py",
    )
    parser.add_argument(
        "--check-inputs",
        action="store_true",
        help="preflight every referenced bundle input path before executing; useful with --dry-run",
    )
    parser.add_argument(
        "--write-template",
        help="write a canonical s7_production_bundle_manifest.v1 skeleton and exit",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    if args.write_template is not None:
        write_template(Path(args.write_template))
        return 0
    if args.manifest is None:
        parser.error("--manifest is required unless --write-template is used")

    manifest_path = Path(args.manifest).resolve()
    try:
        manifest = load_manifest(manifest_path)
        commands = build_commands(
            manifest,
            manifest_path.parent,
            root,
            args.cargo,
            args.verify_mode,
            review_options=review_options_from_args(args, root),
        )
    except AssembleError as error:
        print("S7 packet assembly: NEEDS_CHANGES")
        print(f" - {error}")
        return 1

    if args.check_inputs or not args.dry_run:
        missing = missing_input_paths(commands)
        if missing:
            print("S7 packet assembly: NEEDS_CHANGES")
            for path in missing:
                print(f" - missing input file: {path}")
            return 1

    for command in commands:
        print("+ " + shlex.join(command))
        if not args.dry_run:
            completed = subprocess.run(command, check=False)
            if completed.returncode != 0:
                print("S7 packet assembly: NEEDS_CHANGES")
                print(f" - command failed with exit {completed.returncode}: {shlex.join(command)}")
                return completed.returncode

    print("S7 packet assembly: dry-run ok" if args.dry_run else "S7 packet assembly: ok")
    return 0


def load_manifest(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise AssembleError(f"manifest missing: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise AssembleError(f"{path} is not valid JSON: {error}") from error
    if not isinstance(payload, dict):
        raise AssembleError(f"{path} must contain a JSON object")
    if payload.get("schema") != SCHEMA:
        raise AssembleError(f"manifest schema must be {SCHEMA!r}")
    return payload


def build_commands(
    manifest: dict[str, Any],
    manifest_dir: Path,
    root: Path,
    cargo: str,
    verify_mode: str,
    review_options: ReviewOptions | None = None,
) -> list[list[str]]:
    validate_known_fields(manifest)
    commands: list[list[str]] = []
    for topology in TOPOLOGIES:
        topology_runs = require_object(manifest, ["runs", topology])
        for seed in SEEDS:
            run = require_object(topology_runs, [str(seed)], f"runs.{topology}.{seed}")
            for field in REQUIRED_RUN_FIELDS:
                require_string(run, [field], f"runs.{topology}.{seed}.{field}")
            commands.append(
                gbf_command(cargo)
                + [
                    "materialize-run",
                    "--root",
                    str(root),
                    "--topology",
                    topology,
                    "--seed",
                    str(seed),
                    "--run-log",
                    str(bundle_path(manifest_dir, run["run_log"])),
                    "--score",
                    str(bundle_path(manifest_dir, run["score"])),
                    "--grad-log",
                    str(bundle_path(manifest_dir, run["grad_log"])),
                    "--router-step-telemetry",
                    str(bundle_path(manifest_dir, run["router_step_telemetry"])),
                ]
            )

    switch_stats = require_object(manifest, ["switch_stats"])
    for seed in SEEDS:
        path = require_string(switch_stats, [str(seed)], f"switch_stats.{seed}")
        commands.append(
            support_command(cargo, root, "switch-stats", bundle_path(manifest_dir, path))
            + ["--seed", str(seed)]
        )

    support = require_object(manifest, ["support_artifacts"])
    for field in REQUIRED_SUPPORT_ARTIFACTS:
        require_string(support, [field], f"support_artifacts.{field}")
    commands.extend(
        [
            support_command(
                cargo,
                root,
                "router-collapse-sweep",
                bundle_path(manifest_dir, support["router_collapse_sweep"]),
            ),
            support_command(
                cargo,
                root,
                "burn-grad-smoke",
                bundle_path(manifest_dir, support["burn_grad_smoke"]),
            ),
            support_command(
                cargo,
                root,
                "oracle-routed",
                bundle_path(manifest_dir, support["oracle_routed"]),
            ),
            support_command(
                cargo,
                root,
                "emulator-one-token",
                bundle_path(manifest_dir, support["emulator_one_token_moe"]),
            )
            + ["--topology", "MoeTiny"],
        ]
    )
    dense_emulator = support.get("emulator_one_token_dense")
    if dense_emulator is not None:
        if not isinstance(dense_emulator, str) or not dense_emulator.strip():
            raise AssembleError("support_artifacts.emulator_one_token_dense must be a non-empty string")
        commands.append(
            support_command(
                cargo,
                root,
                "emulator-one-token",
                bundle_path(manifest_dir, dense_emulator),
            )
            + ["--topology", "MoeTinyDenseMatched"]
        )

    comparison = require_object(manifest, ["comparison"])
    moe_topology_hash = require_hash(comparison, ["moe_topology_hash"], "comparison.moe_topology_hash")
    dense_topology_hash = require_hash(
        comparison,
        ["dense_matched_topology_hash"],
        "comparison.dense_matched_topology_hash",
    )
    commands.append(gbf_command(cargo) + ["derive-summaries", "--root", str(root)])
    commands.append(
        gbf_command(cargo)
        + [
            "derive-comparison",
            "--root",
            str(root),
            "--moe-topology-hash",
            moe_topology_hash,
            "--dense-matched-topology-hash",
            dense_topology_hash,
        ]
    )

    frontier = require_object(manifest, ["frontier"])
    commands.append(frontier_command(cargo, root, manifest_dir, frontier))

    report = require_object(manifest, ["report"])
    commands.append(report_command(cargo, root, report))
    if review_options is not None:
        commands.append(review_command(root, review_options))

    verify = [str(root / "scripts/review/f-s7/verify-packet.sh")]
    if verify_mode == "skip-gates":
        verify.append("--skip-gates")
    commands.append(verify)
    return commands


class ReviewOptions:
    def __init__(
        self,
        *,
        acpx: str,
        review_cwd: str,
        timeout: str,
        reviewer: str,
        gemini_agent: str | None,
        claude_agent: str | None,
    ) -> None:
        self.acpx = acpx
        self.review_cwd = review_cwd
        self.timeout = timeout
        self.reviewer = reviewer
        self.gemini_agent = gemini_agent
        self.claude_agent = claude_agent


def review_options_from_args(args: argparse.Namespace, root: Path) -> ReviewOptions | None:
    if not args.run_reviews:
        return None
    review_cwd = args.review_cwd or str(root)
    return ReviewOptions(
        acpx=args.acpx,
        review_cwd=review_cwd,
        timeout=args.review_timeout,
        reviewer=args.reviewer,
        gemini_agent=args.gemini_agent,
        claude_agent=args.claude_agent,
    )


def review_command(root: Path, options: ReviewOptions) -> list[str]:
    command = [
        str(root / "scripts/review/f-s7/run-acpx-reviews.py"),
        "--root",
        str(root),
        "--review-cwd",
        options.review_cwd,
        "--acpx",
        options.acpx,
        "--timeout",
        options.timeout,
        "--reviewer",
        options.reviewer,
    ]
    if options.gemini_agent is not None:
        if not options.gemini_agent.strip():
            raise AssembleError("--gemini-agent must be a non-empty string")
        command.extend(["--gemini-agent", options.gemini_agent])
    if options.claude_agent is not None:
        if not options.claude_agent.strip():
            raise AssembleError("--claude-agent must be a non-empty string")
        command.extend(["--claude-agent", options.claude_agent])
    return command


def write_template(path: Path) -> None:
    runs: dict[str, dict[str, dict[str, str]]] = {}
    for topology in TOPOLOGIES:
        runs[topology] = {}
        for seed in SEEDS:
            base = f"runs/{topology}/seed-{seed}"
            runs[topology][str(seed)] = {
                "run_log": f"{base}/run-log.json",
                "score": f"{base}/score.json",
                "grad_log": f"{base}/grad-log.jsonl",
                "router_step_telemetry": f"{base}/router-step-telemetry.jsonl",
            }

    payload = {
        "schema": SCHEMA,
        "runs": runs,
        "switch_stats": {
            str(seed): f"switch-stats/seed-{seed}/switch-stats.json" for seed in SEEDS
        },
        "support_artifacts": {
            "router_collapse_sweep": "router-collapse/seed-0/sweep.json",
            "burn_grad_smoke": "burn-grad-smoke/expert_block_qat.json",
            "oracle_routed": "oracle-routed/seed-0/oracle.json",
            "emulator_one_token_moe": "emulator-one-token/seed-0/MoeTiny/result.json",
            "emulator_one_token_dense": "emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
        },
        "comparison": {
            "moe_topology_hash": "sha256:" + "1" * 64,
            "dense_matched_topology_hash": "sha256:" + "2" * 64,
        },
        "frontier": {
            "moe_conformance": "frontier/moe-conformance.json",
            "dense_conformance": "frontier/dense-conformance.json",
            "moe_deployed_bytes_per_block": [20944, 20944, 20944, 20944],
            "dense_deployed_bytes_per_block": [20948, 20948, 20948, 20948],
            "moe_schedule_cost": "frontier/moe-schedule-cost.json",
            "dense_schedule_cost": "frontier/dense-schedule-cost.json",
        },
        "report": {
            "s7_outcome": "PassClean",
            "decision": "ProceedToS8",
            "predictions_section_hash": "sha256:" + "3" * 64,
            "predictions_commit": "4" * 40,
            "first_result_commit": "5" * 40,
            "rfc_revision": "6" * 40,
            "generated_at": "2026-06-25T00:00:00Z",
        },
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(f"S7 packet assembly: wrote manifest template {path}")


def validate_known_fields(manifest: dict[str, Any]) -> None:
    reject_unknown("manifest", manifest, TOP_LEVEL_FIELDS)
    runs = require_object(manifest, ["runs"])
    reject_unknown("runs", runs, set(TOPOLOGIES))
    for topology in TOPOLOGIES:
        topology_runs = require_object(runs, [topology], f"runs.{topology}")
        seed_keys = {str(seed) for seed in SEEDS}
        reject_unknown(f"runs.{topology}", topology_runs, seed_keys)
        for seed in SEEDS:
            run = require_object(topology_runs, [str(seed)], f"runs.{topology}.{seed}")
            reject_unknown(f"runs.{topology}.{seed}", run, set(REQUIRED_RUN_FIELDS))
    switch_stats = require_object(manifest, ["switch_stats"])
    reject_unknown("switch_stats", switch_stats, {str(seed) for seed in SEEDS})
    support = require_object(manifest, ["support_artifacts"])
    reject_unknown("support_artifacts", support, SUPPORT_FIELDS)
    comparison = require_object(manifest, ["comparison"])
    reject_unknown("comparison", comparison, COMPARISON_FIELDS)
    frontier = require_object(manifest, ["frontier"])
    reject_unknown("frontier", frontier, FRONTIER_FIELDS)
    report = require_object(manifest, ["report"])
    reject_unknown("report", report, REPORT_FIELDS)


def reject_unknown(label: str, payload: dict[str, Any], allowed: set[str]) -> None:
    unknown = sorted(set(payload) - allowed)
    if unknown:
        raise AssembleError(f"{label} has unknown field(s): {', '.join(unknown)}")


def gbf_command(cargo: str) -> list[str]:
    return [
        cargo,
        "run",
        "-q",
        "-p",
        "gbf-cli",
        "--no-default-features",
        "--features",
        "s7",
        "--",
        "--log-level",
        "off",
        "s7",
    ]


def support_command(cargo: str, root: Path, kind: str, path: Path) -> list[str]:
    return gbf_command(cargo) + [
        "materialize-support-artifact",
        "--root",
        str(root),
        "--kind",
        kind,
        "--input",
        str(path),
    ]


def frontier_command(
    cargo: str, root: Path, manifest_dir: Path, frontier: dict[str, Any]
) -> list[str]:
    moe_conformance = bundle_path(
        manifest_dir, require_string(frontier, ["moe_conformance"], "frontier.moe_conformance")
    )
    dense_conformance = bundle_path(
        manifest_dir,
        require_string(frontier, ["dense_conformance"], "frontier.dense_conformance"),
    )
    moe_bytes = require_u64_list(
        frontier,
        ["moe_deployed_bytes_per_block"],
        "frontier.moe_deployed_bytes_per_block",
    )
    dense_bytes = require_u64_list(
        frontier,
        ["dense_deployed_bytes_per_block"],
        "frontier.dense_deployed_bytes_per_block",
    )
    command = gbf_command(cargo) + [
        "derive-frontier",
        "--root",
        str(root),
        "--moe-conformance",
        str(moe_conformance),
        "--dense-conformance",
        str(dense_conformance),
        "--moe-deployed-bytes-per-block",
        ",".join(str(item) for item in moe_bytes),
        "--dense-deployed-bytes-per-block",
        ",".join(str(item) for item in dense_bytes),
    ]
    for key, flag in [
        ("moe_schedule_cost", "--moe-schedule-cost"),
        ("dense_schedule_cost", "--dense-schedule-cost"),
    ]:
        value = frontier.get(key)
        if value is not None:
            if not isinstance(value, str) or not value.strip():
                raise AssembleError(f"frontier.{key} must be a non-empty string")
            command.extend([flag, str(bundle_path(manifest_dir, value))])
    return command


def report_command(cargo: str, root: Path, report: dict[str, Any]) -> list[str]:
    outcome = require_string(report, ["s7_outcome"], "report.s7_outcome")
    if outcome not in {"PassClean", "FailParity"}:
        raise AssembleError("report.s7_outcome must be PassClean or FailParity")
    decision = require_report_decision(report, outcome)
    rfc_revision = require_report_revision(report)
    predictions_section_hash = require_hash(
        report,
        ["predictions_section_hash"],
        "report.predictions_section_hash",
    )
    predictions_commit = require_commit(report, ["predictions_commit"], "report.predictions_commit")
    first_result_commit = require_commit(
        report,
        ["first_result_commit"],
        "report.first_result_commit",
    )
    command = gbf_command(cargo) + [
        "emit-report",
        "--root",
        str(root),
        "--s7-outcome",
        outcome,
        "--predictions-section-hash",
        predictions_section_hash,
        "--predictions-commit",
        predictions_commit,
        "--first-result-commit",
        first_result_commit,
    ]
    command.extend(["--decision", decision])
    command.extend(["--rfc-revision", rfc_revision])
    for key, flag in [("generated_at", "--generated-at")]:
        value = report.get(key)
        if value is not None:
            if not isinstance(value, str) or not value.strip():
                raise AssembleError(f"report.{key} must be a non-empty string")
            command.extend([flag, value])
    return command


def require_report_decision(report: dict[str, Any], outcome: str) -> str:
    value = require_string(report, ["decision"], "report.decision")
    if value not in {"ProceedToS8", "ProceedToS8DenseOnly"}:
        raise AssembleError("report.decision must be ProceedToS8 or ProceedToS8DenseOnly")
    if outcome == "PassClean" and value != "ProceedToS8":
        raise AssembleError("report.decision must be ProceedToS8 when report.s7_outcome is PassClean")
    if outcome == "FailParity" and value != "ProceedToS8DenseOnly":
        raise AssembleError(
            "report.decision must be ProceedToS8DenseOnly when report.s7_outcome is FailParity"
        )
    return value


def require_report_revision(report: dict[str, Any]) -> str:
    value = require_string(report, ["rfc_revision"], "report.rfc_revision")
    if not (COMMIT_RE.match(value) or HASH_RE.match(value)):
        raise AssembleError("report.rfc_revision must be a 40-hex git commit id or sha256 hash")
    return value


def input_paths_for_command(command: list[str]) -> list[Path]:
    paths: list[Path] = []
    for flag in [
        "--run-log",
        "--score",
        "--grad-log",
        "--router-step-telemetry",
        "--input",
        "--moe-conformance",
        "--dense-conformance",
        "--moe-schedule-cost",
        "--dense-schedule-cost",
    ]:
        for index, item in enumerate(command[:-1]):
            if item == flag:
                paths.append(Path(command[index + 1]))
    return paths


def missing_input_paths(commands: list[list[str]]) -> list[Path]:
    seen: set[Path] = set()
    missing: list[Path] = []
    for command in commands:
        for path in input_paths_for_command(command):
            if path in seen:
                continue
            seen.add(path)
            if not path.is_file():
                missing.append(path)
    return missing


def bundle_path(manifest_dir: Path, value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return (manifest_dir / path).resolve()


def require_object(
    payload: dict[str, Any], keys: list[str], label: str | None = None
) -> dict[str, Any]:
    value: Any = payload
    for key in keys:
        if not isinstance(value, dict):
            raise AssembleError(f"{label or '.'.join(keys)} must be an object")
        value = value.get(key)
    if not isinstance(value, dict):
        raise AssembleError(f"{label or '.'.join(keys)} must be an object")
    return value


def require_string(payload: dict[str, Any], keys: list[str], label: str) -> str:
    value: Any = payload
    for key in keys:
        if not isinstance(value, dict):
            raise AssembleError(f"{label} must be a non-empty string")
        value = value.get(key)
    if not isinstance(value, str) or not value.strip():
        raise AssembleError(f"{label} must be a non-empty string")
    return value


def require_hash(payload: dict[str, Any], keys: list[str], label: str) -> str:
    value = require_string(payload, keys, label)
    if not HASH_RE.match(value):
        raise AssembleError(f"{label} must be a sha256 hash")
    return value


def require_commit(payload: dict[str, Any], keys: list[str], label: str) -> str:
    value = require_string(payload, keys, label)
    if not COMMIT_RE.match(value):
        raise AssembleError(f"{label} must be a 40-hex git commit id")
    return value


def require_u64_list(payload: dict[str, Any], keys: list[str], label: str) -> list[int]:
    value: Any = payload
    for key in keys:
        if not isinstance(value, dict):
            raise AssembleError(f"{label} must be a non-empty list of positive integers")
        value = value.get(key)
    if not isinstance(value, list) or not value:
        raise AssembleError(f"{label} must be a non-empty list of positive integers")
    values: list[int] = []
    for item in value:
        if not isinstance(item, int) or item <= 0:
            raise AssembleError(f"{label} must be a non-empty list of positive integers")
        values.append(item)
    return values


class AssembleError(RuntimeError):
    pass


if __name__ == "__main__":
    sys.exit(main())
