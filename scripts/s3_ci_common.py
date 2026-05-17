#!/usr/bin/env python3
"""Shared F-S3 CI gate runner used by scripts/s3_*_check.sh."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback.
    tomllib = None


SCRIPT_CONFIG = {
    "s3_preregistration_check": "s3-preregistration",
    "s3_determinism_check": "s3-determinism",
    "s3_full_determinism_check": "s3-full-determinism",
    "s3_isolation_check": "s3-isolation",
    "s3_api_drift_check": "s3-api-drift",
    "s3_oracle_re_run_check": "s3-oracle-re-run",
    "s3_no_naming_resolution_check": "s3-no-naming-resolution",
    "s3_feature_matrix_check": "s3-feature-matrix",
}

ROOT = Path(__file__).resolve().parents[1]
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
RESULT_HASH_RE = re.compile(
    rb'"(?:charset_self_hash|baseline_self_hash|workload_self_hash|'
    rb'bundle_self_hash|artifact_self_hash|agreement_self_hash|'
    rb'conformance_self_hash|v0_success_self_hash)"\s*:\s*"sha256:[0-9a-f]{64}"'
)


class GateFailure(Exception):
    def __init__(self, stage: int, reason: str):
        super().__init__(reason)
        self.stage = stage
        self.reason = reason


class Gate:
    def __init__(self, script: str, dry_run: bool, report_path: Path):
        self.script = script
        self.dry_run = dry_run
        self.report_path = report_path
        self.stages: list[dict[str, Any]] = []

    def emit(self, payload: dict[str, Any]) -> None:
        print(json.dumps(payload, sort_keys=True, separators=(",", ":")), file=sys.stderr)

    def stage_start(self, index: int, description: str) -> None:
        self.emit(
            {
                "event": f"{self.script}_stage_start",
                "stage": index,
                "description": description,
            }
        )

    def stage_done(self, name: str, index: int, passed: bool, detail: dict[str, Any]) -> None:
        row = {"name": name, "passed": passed, "detail": detail}
        self.stages.append(row)
        self.emit(
            {
                "event": f"{self.script}_stage_done",
                "stage": index,
                "passed": passed,
                "detail": detail,
            }
        )

    def finish(self, passed: bool, exit_code: int, summary: str) -> int:
        report = {
            "script": self.script,
            "passed": passed,
            "stages": self.stages,
            "exit_code": exit_code,
            "dry_run": self.dry_run,
            "evidence_mode": "dry_run" if self.dry_run else "live",
            "live_evidence": not self.dry_run,
        }
        self.report_path.parent.mkdir(parents=True, exist_ok=True)
        self.report_path.write_text(
            json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n",
            encoding="utf-8",
        )
        self.emit({"event": f"{self.script}_summary", **report, "summary": summary})
        print(summary)
        return exit_code


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[1] not in SCRIPT_CONFIG:
        known = ", ".join(sorted(SCRIPT_CONFIG))
        print(f"usage: s3_ci_common.py <script> [args...]\nknown scripts: {known}", file=sys.stderr)
        return 2
    script = argv[1]
    slug = SCRIPT_CONFIG[script]
    parser = argparse.ArgumentParser(prog=f"scripts/{script}.sh")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--report-path", default=f"/tmp/{slug}.json")
    parser.add_argument("--report-dir")
    parser.add_argument("--pin", default="experiments/S3/preregistration.toml")
    parser.add_argument("--rfc", default="history/rfcs/F-S3-v0-success-tinystories.md")
    parser.add_argument(
        "--result-state",
        choices=("pre", "post"),
        default="pre",
        help="verify either the default pre-result preregistration state or the post-result registered state",
    )
    parser.add_argument("--artifact-dir", action="append", default=None)
    parser.add_argument("--force-failure-for-test", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("target", nargs="?")
    args = parser.parse_args(argv[2:])
    report_path = Path(args.report_path)
    if args.report_dir:
        report_path = Path(args.report_dir) / f"{slug}.json"
    gate = Gate(script, args.dry_run, report_path)
    try:
        run_gate(script, args, gate)
        return gate.finish(
            True,
            0,
            f"S3 {slug} PASS dry_run={str(args.dry_run).lower()} report={report_path}",
        )
    except GateFailure as failure:
        mark_failed_stage(gate, failure)
        return gate.finish(
            False,
            1,
            f"S3 {slug} FAIL stage={failure.stage} reason={failure.reason} report={report_path}",
        )
    except Exception as error:  # Keep shell failures structured, not tracebacks.
        failure = GateFailure(len(gate.stages) + 1, str(error))
        mark_failed_stage(gate, failure)
        return gate.finish(
            False,
            1,
            f"S3 {slug} FAIL stage={failure.stage} reason={failure.reason} report={report_path}",
        )


def mark_failed_stage(gate: Gate, failure: GateFailure) -> None:
    if gate.stages and gate.stages[-1]["passed"]:
        detail = dict(gate.stages[-1]["detail"])
        detail["reason"] = failure.reason
        gate.stages[-1] = {**gate.stages[-1], "passed": False, "detail": detail}


def run_gate(script: str, args: argparse.Namespace, gate: Gate) -> None:
    {
        "s3_preregistration_check": preregistration_check,
        "s3_determinism_check": determinism_check,
        "s3_full_determinism_check": full_determinism_check,
        "s3_isolation_check": isolation_check,
        "s3_api_drift_check": api_drift_check,
        "s3_oracle_re_run_check": oracle_re_run_check,
        "s3_no_naming_resolution_check": no_naming_resolution_check,
        "s3_feature_matrix_check": feature_matrix_check,
    }[script](args, gate)
    if args.force_failure_for_test:
        stage = len(gate.stages) if gate.stages else 1
        raise GateFailure(stage, "forced failure for script plumbing regression")


def preregistration_check(args: argparse.Namespace, gate: Gate) -> None:
    pin_path = resolve(args.pin)
    rfc_path = resolve(args.rfc)
    artifact_dirs = [resolve(path) for path in (args.artifact_dir or ["experiments/S3"])]

    gate.stage_start(1, "validate preregistration pin and predictions hash")
    pin = load_pin(pin_path)
    section_hash = predictions_hash(predictions_section(rfc_path.read_text(encoding="utf-8")))
    detail = {
        "pin": rel(pin_path),
        "rfc": rel(rfc_path),
        "predictions_section_hash": section_hash,
        "pin_predictions_section_hash": pin["predictions_section_hash"],
    }
    passed = section_hash == pin["predictions_section_hash"]
    gate.stage_done("predictions_hash", 1, passed, detail)
    if not passed:
        raise GateFailure(1, "predictions_section_hash mismatch")
    for field in ("predictions_commit", "rfc_revision"):
        if not COMMIT_RE.fullmatch(pin[field]):
            raise GateFailure(1, f"{field} must be a lowercase 40-character git commit id")
    if args.result_state == "pre":
        if pin["first_result_commit"] != "":
            raise GateFailure(1, "first_result_commit must remain empty before first S3 result")
    else:
        if pin["first_result_commit"] == "":
            raise GateFailure(1, "first_result_commit is required in post-result mode")
        if not COMMIT_RE.fullmatch(pin["first_result_commit"]):
            raise GateFailure(1, "first_result_commit must be a lowercase 40-character git commit id")
        if pin["predictions_commit"] == pin["first_result_commit"] or not git_is_ancestor(
            pin["predictions_commit"], pin["first_result_commit"]
        ):
            raise GateFailure(1, "predictions_commit must be a strict ancestor of first_result_commit")

    gate.stage_start(
        2,
        "scan for preregistration-breaking result artifacts"
        if args.result_state == "pre"
        else "scan for registered S3 result artifacts",
    )
    found = first_result_artifact_path(pin_path, artifact_dirs)
    passed = found is None if args.result_state == "pre" else found is not None
    gate.stage_done(
        "empty_result_scan" if args.result_state == "pre" else "registered_result_scan",
        2,
        passed,
        {"artifact_dirs": [rel(path) for path in artifact_dirs], "first_result_artifact": found},
    )
    if args.result_state == "pre" and found is not None:
        raise GateFailure(2, f"unexpected S3 result artifact evidence in {found}")
    if args.result_state == "post" and found is None:
        raise GateFailure(2, "missing S3 result artifact evidence in post-result mode")


def determinism_check(args: argparse.Namespace, gate: Gate) -> None:
    replay_compare(args, gate, [0], "determinism_smoke")


def full_determinism_check(args: argparse.Namespace, gate: Gate) -> None:
    replay_compare(args, gate, [0, 1, 2, 3, 4], "full_determinism")


def replay_compare(args: argparse.Namespace, gate: Gate, seeds: list[int], name: str) -> None:
    gate.stage_start(1, "validate B23 CLI replay surface")
    cli = ROOT / "gbf-cli" / "src" / "main.rs"
    gate.stage_done("inputs", 1, cli.exists(), {"gbf_cli_exists": cli.exists(), "seeds": seeds})
    if not cli.exists():
        raise GateFailure(1, "missing gbf-cli S3 dispatch")
    gate.stage_start(2, "run replay 1")
    first = None if args.dry_run else run_replay(seeds, "s3-full")
    gate.stage_done(
        "replay_1",
        2,
        True,
        replay_detail(first, seeds, "gbf s3 replay-full"),
    )
    gate.stage_start(3, "run replay 2")
    second = None if args.dry_run else run_replay(seeds, "s3-full")
    gate.stage_done(
        "replay_2",
        3,
        True,
        replay_detail(second, seeds, "gbf s3 replay-full"),
    )
    gate.stage_start(4, "compare replay evidence bytes")
    passed = args.dry_run or first == second
    gate.stage_done(
        "bytewise_compare",
        4,
        passed,
        {
            "comparison": name,
            "seed_list": seeds,
            "first_sha": None if first is None else sha256_bytes(first),
            "second_sha": None if second is None else sha256_bytes(second),
        },
    )
    if not passed:
        raise GateFailure(4, "S3 replay evidence mismatch")


def isolation_check(args: argparse.Namespace, gate: Gate) -> None:
    seeds = [0, 1, 2, 3, 4]
    gate.stage_start(1, "collect real-oracle replay evidence")
    real = None if args.dry_run else run_replay(seeds, "s3-full")
    real_payload = None if real is None else json.loads(real)
    gate.stage_done("real_oracle_replay", 1, True, replay_detail(real, seeds, "gbf s3 replay-full"))

    gate.stage_start(2, "assert at least two seeds differ")
    hashes = [] if real_payload is None else per_seed_bundle_artifact_hashes(real_payload)
    distinct = 2 if args.dry_run else len(set(hashes))
    gate.stage_done(
        "seed_distinctness",
        2,
        distinct >= 2,
        {"distinct_bundle_artifact_hash_count": distinct, "seed_count": len(seeds)},
    )
    if distinct < 2:
        raise GateFailure(2, "seed bundle/artifact hashes are not distinct")

    gate.stage_start(3, "compare real vs fallback oracle training artifacts")
    fallback = None if args.dry_run else run_replay(seeds, "s3,s3-phase-d,s3-oracle-fallback", fallback=True)
    fallback_payload = None if fallback is None else json.loads(fallback)
    same = args.dry_run or per_seed_bundle_artifact_hashes(real_payload) == per_seed_bundle_artifact_hashes(fallback_payload)
    gate.stage_done(
        "real_fallback_training_hash_parity",
        3,
        same,
        {
            "dry_run": args.dry_run,
            "real_schema": None if real_payload is None else real_payload.get("schema"),
            "fallback_schema": None if fallback_payload is None else fallback_payload.get("schema"),
        },
    )
    if not same:
        raise GateFailure(3, "real/fallback oracle changed bundle or artifact hashes")


def api_drift_check(args: argparse.Namespace, gate: Gate) -> None:
    snapshots = {
        "gbf-artifact": ROOT / "gbf-experiments/snapshots/s3_artifact_public_api.txt",
        "gbf-workload": ROOT / "gbf-experiments/snapshots/s3_workload_public_api.txt",
        "gbf-oracle": ROOT / "gbf-experiments/snapshots/s3_oracle_public_api.txt",
    }
    gate.stage_start(1, "validate S3 public API snapshots exist")
    missing = [crate for crate, path in snapshots.items() if not path.exists()]
    gate.stage_done("snapshot_inputs", 1, not missing, {"missing": missing})
    if missing:
        raise GateFailure(1, f"missing S3 API snapshots: {missing}")
    gate.stage_start(2, "compare public API snapshots")
    drift = {}
    for crate, snapshot in snapshots.items():
        observed = public_api_surface(ROOT / crate / "src/lib.rs")
        expected = snapshot.read_text(encoding="utf-8").splitlines()
        if observed != expected:
            drift[crate] = {"expected": expected, "observed": observed}
    passed = args.dry_run or not drift
    gate.stage_done(
        "api_drift_compare",
        2,
        passed,
        {"snapshot_count": len(snapshots), "drift_crates": sorted(drift)},
    )
    if not passed:
        raise GateFailure(2, f"S3 public API drift detected: {sorted(drift)}")


def oracle_re_run_check(args: argparse.Namespace, gate: Gate) -> None:
    gate.stage_start(1, "validate oracle re-run CLI surface")
    gate.stage_done("inputs", 1, True, {"command": "gbf s3 oracle-re-run"})
    gate.stage_start(2, "run inherited oracle re-run")
    payload = None
    if not args.dry_run:
        with tempfile.TemporaryDirectory(prefix="s3-oracle-re-run-") as tmp:
            report = Path(tmp) / "oracle-re-run.json"
            run_cmd(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "gbf-cli",
                    "--features",
                    "s3-full",
                    "--",
                    "s3",
                    "oracle-re-run",
                    "--output",
                    str(report),
                ]
            )
            payload = json.loads(report.read_text(encoding="utf-8"))
    passed = args.dry_run or payload.get("schema") == "s3_oracle_re_run.v1"
    gate.stage_done(
        "oracle_re_run",
        2,
        passed,
        {"schema": None if payload is None else payload.get("schema")},
    )
    if not passed:
        raise GateFailure(2, "oracle re-run payload schema mismatch")


def no_naming_resolution_check(args: argparse.Namespace, gate: Gate) -> None:
    target = resolve(args.target or "experiments/S3/artifacts")
    gate.stage_start(1, "discover S3 artifact metadata files")
    paths = artifact_metadata_paths(target)
    gate.stage_done("discover_metadata", 1, True, {"target": rel(target), "metadata_count": len(paths)})
    gate.stage_start(2, "assert tensors_resolved_via_naming is zero")
    offenders = []
    if not args.dry_run:
        for path in paths:
            data = json.loads(path.read_text(encoding="utf-8"))
            if data.get("schema") == "s3_artifact.v1":
                observed = data.get("weight_resolution_summary", {}).get("tensors_resolved_via_naming")
                if observed != 0:
                    offenders.append({"path": rel(path), "observed": observed})
    gate.stage_done("no_naming_resolution", 2, not offenders, {"offenders": offenders})
    if offenders:
        raise GateFailure(2, "artifact metadata used naming-based tensor resolution")


def feature_matrix_check(args: argparse.Namespace, gate: Gate) -> None:
    matrix = [
        ("s3", "build"),
        ("s3,s3-phase-d", "build"),
        ("s3,s3-phase-d,s3-oracle-real", "build"),
        ("s3,s3-phase-d,s3-oracle-fallback", "build"),
        ("s3,s3-phase-d,s3-oracle-real,falsify", "test-no-run"),
    ]
    gate.stage_start(1, "validate feature matrix")
    gate.stage_done("matrix_plan", 1, True, {"entries": [{"features": f, "mode": m} for f, m in matrix]})
    if args.dry_run:
        return
    for index, (features, mode) in enumerate(matrix, start=2):
        gate.stage_start(index, f"cargo {mode} {features}")
        if mode == "test-no-run":
            command = ["cargo", "test", "-p", "gbf-experiments", "--no-run", "--no-default-features", "--features", features]
        else:
            command = ["cargo", "build", "-p", "gbf-experiments", "--no-default-features", "--features", features]
        run_cmd(command)
        gate.stage_done(f"feature_{features}", index, True, {"features": features, "mode": mode})


def run_replay(seeds: list[int], features: str, fallback: bool = False) -> bytes:
    with tempfile.TemporaryDirectory(prefix="s3-replay-") as tmp:
        output = Path(tmp) / "replay.json"
        command = [
            "cargo",
            "run",
            "-q",
            "-p",
            "gbf-cli",
            "--no-default-features",
            "--features",
            features,
            "--",
            "s3",
            "replay-fallback" if fallback else "replay-full",
            "--seed-list",
            ",".join(str(seed) for seed in seeds),
            "--output",
            str(output),
        ]
        run_cmd(command)
        return output.read_bytes()


def replay_detail(payload: bytes | None, seeds: list[int], source: str) -> dict[str, Any]:
    if payload is None:
        return {"dry_run": True, "seed_list": seeds, "evidence_source": source, "schema": None}
    data = json.loads(payload)
    return {
        "dry_run": False,
        "seed_list": seeds,
        "evidence_source": data.get("evidence_source"),
        "schema": data.get("schema"),
        "per_seed_count": len(data.get("per_seed", [])),
        "payload_sha": sha256_bytes(payload),
    }


def per_seed_bundle_artifact_hashes(payload: dict[str, Any]) -> list[tuple[str, str]]:
    return [
        (row["bundle_self_hash"], row["artifact_self_hash"])
        for row in sorted(payload["per_seed"], key=lambda row: row["seed"])
    ]


def artifact_metadata_paths(target: Path) -> list[Path]:
    if target.is_file():
        return [target]
    if not target.exists():
        return []
    return sorted(target.rglob("artifact-metadata.json"))


def public_api_surface(path: Path) -> list[str]:
    lines = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if stripped.startswith("pub mod ") or stripped.startswith("pub use ") or stripped.startswith("pub const "):
            lines.append(stripped.rstrip(";"))
    return sorted(lines)


def load_pin(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    data = tomllib.loads(text) if tomllib is not None else parse_string_toml(text)
    required = {
        "schema",
        "predictions_commit",
        "predictions_section_hash",
        "pass_version_S3",
        "rfc_revision",
        "first_result_commit",
    }
    missing = sorted(required.difference(data))
    if missing:
        raise GateFailure(1, f"pin is missing required fields: {', '.join(missing)}")
    if data["schema"] != "s3_preregistration.v1":
        raise GateFailure(1, "pin schema must be s3_preregistration.v1")
    if not HASH_RE.fullmatch(str(data["predictions_section_hash"])):
        raise GateFailure(1, "predictions_section_hash must be sha256:<64 lowercase hex>")
    return data


def parse_string_toml(text: str) -> dict[str, str]:
    data = {}
    for line in text.splitlines():
        stripped = line.split("#", 1)[0].strip()
        if not stripped:
            continue
        key, raw = stripped.split("=", 1)
        data[key.strip()] = json.loads(raw.strip())
    return data


def predictions_section(markdown: str) -> str:
    pairs = [
        ("## Pre-registered predictions\n\n", "\n## Observed\n"),
        ("  ## Pre-registered predictions\n", "\n\n  ## Observed\n"),
    ]
    for start_marker, end_marker in pairs:
        start = markdown.find(start_marker)
        if start < 0:
            continue
        start += len(start_marker)
        end = markdown.find(end_marker, start)
        if end >= 0:
            return markdown[start:end].strip()
    raise GateFailure(1, "missing Pre-registered predictions section followed by Observed")


def predictions_hash(section: str) -> str:
    canonical = json.dumps(section.strip(), sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def git_is_ancestor(ancestor: str, descendant: str) -> bool:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return completed.returncode == 0


def first_result_artifact_path(pin_path: Path, artifact_dirs: list[Path]) -> str | None:
    for artifact_dir in artifact_dirs:
        if not artifact_dir.exists():
            continue
        for path in sorted(artifact_dir.rglob("*")):
            if path.is_file() and path != pin_path and RESULT_HASH_RE.search(path.read_bytes()):
                return rel(path)
    return None


def run_cmd(command: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            json.dumps(
                {
                    "command": command,
                    "returncode": completed.returncode,
                    "stdout_tail": completed.stdout[-2000:],
                    "stderr_tail": completed.stderr[-4000:],
                },
                sort_keys=True,
            )
        )
    return completed


def sha256_bytes(payload: bytes) -> str:
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def resolve(raw: str | Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else ROOT / path


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
