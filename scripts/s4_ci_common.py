#!/usr/bin/env python3
"""Shared F-S4 CI gate runner used by scripts/s4_*_check.sh."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback.
    tomllib = None

SCRIPT_CONFIG = {
    "s4_determinism_check": "s4-determinism",
    "s4_full_determinism_check": "s4-full-determinism",
    "s4_isolation_check": "s4-isolation",
    "s4_api_drift_check": "s4-api-drift",
}

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_GUTENBERG_FIXTURE = "fixtures/corpora/gutenberg_smoke.toml"
DEFAULT_BUILD_KIND = "phase_d_continuation"
DEFAULT_DEVICE_PROFILE = "S1CpuDeterministic"
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
RAW_HASH_RE = re.compile(r"^[0-9a-f]{64}$")
S4_HASH_RE = re.compile(
    rb'"(?:gutenberg_manifest_self_hash|manifest_self_hash|checkpoint_self_hash|run_log_self_hash|'
    rb'score_self_hash|fp_reference_self_hash|oracle_agreement_self_hash|'
    rb'corpus_quality_self_hash|corpus_progression_self_hash|'
    rb'contamination_self_hash|promotion_gate_self_hash|'
    rb'baseline_gutenberg_self_hash|report_self_hash|catalog_snapshot_sha256|'
    rb'source_blob_sha256)"\s*:\s*"?(?:sha256:)?[0-9a-f]{64}"?'
)
SEED_RE = re.compile(r"seed[-_/](\d+)|seed-(\d+)|seed_(\d+)")
PIN_FIELDS = {
    "catalog_snapshot_sha256",
    "gutenberg_manifest_self_hash",
    "manifest_self_hash",
    "source_blob_sha256",
    "build_kind",
    "device_profile",
}
RESULT_SELF_HASH_FIELDS = {
    "gutenberg_manifest_self_hash",
    "manifest_self_hash",
    "checkpoint_self_hash",
    "run_log_self_hash",
    "score_self_hash",
    "fp_reference_self_hash",
    "oracle_agreement_self_hash",
    "corpus_quality_self_hash",
    "corpus_progression_self_hash",
    "contamination_self_hash",
    "promotion_gate_self_hash",
    "baseline_gutenberg_self_hash",
    "report_self_hash",
}
EXPECTED_S4_MODULE_SURFACE = [
    "pub mod baseline",
    "pub mod cli",
    "pub mod contamination",
    "pub mod corpus_oracle",
    "pub mod corpus_progression",
    "pub mod corpus_quality",
    "pub mod device_profile",
    "pub mod harvest",
    "pub mod manifest",
    "pub mod oracle",
    "pub mod promote",
    "pub mod report",
    "pub mod rng",
    "pub mod run",
    "pub mod run_artifacts",
    "pub mod schema",
    "pub mod score",
    "pub mod verifier",
]
OPTIONAL_S4_MODULE_SURFACE = {
    "pub mod falsify": "feature-gated s4-falsify public module",
}


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
        self.emit({"event": f"{self.script}_stage_start", "stage": index, "description": description})

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
        print(f"usage: s4_ci_common.py <script> [args...]\nknown scripts: {known}", file=sys.stderr)
        return 2
    script = argv[1]
    slug = SCRIPT_CONFIG[script]
    parser = argparse.ArgumentParser(prog=f"scripts/{script}.sh")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--report-path", default=f"/tmp/{slug}.json")
    parser.add_argument("--report-dir")
    parser.add_argument("--artifact-dir", action="append", default=None)
    parser.add_argument("--gutenberg-fixture", default=DEFAULT_GUTENBERG_FIXTURE)
    parser.add_argument("--gutenberg-manifest")
    parser.add_argument("--seed-list")
    parser.add_argument("--build-kind", default=DEFAULT_BUILD_KIND)
    parser.add_argument("--device-profile", default=DEFAULT_DEVICE_PROFILE)
    parser.add_argument("--force-failure-for-test", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args(argv[2:])
    report_path = Path(args.report_path)
    if args.report_dir:
        report_path = Path(args.report_dir) / f"{slug}.json"
    gate = Gate(script, args.dry_run, report_path)
    try:
        run_gate(script, args, gate)
        if args.force_failure_for_test:
            raise GateFailure(len(gate.stages) if gate.stages else 1, "forced failure for script plumbing regression")
        return gate.finish(True, 0, f"S4 {slug} PASS dry_run={str(args.dry_run).lower()} report={report_path}")
    except GateFailure as failure:
        mark_failed_stage(gate, failure)
        return gate.finish(
            False,
            1,
            f"S4 {slug} FAIL stage={failure.stage} reason={failure.reason} report={report_path}",
        )
    except Exception as error:  # Keep shell failures structured, not tracebacks.
        failure = GateFailure(len(gate.stages) + 1, str(error))
        mark_failed_stage(gate, failure)
        return gate.finish(
            False,
            1,
            f"S4 {slug} FAIL stage={failure.stage} reason={failure.reason} report={report_path}",
        )


def mark_failed_stage(gate: Gate, failure: GateFailure) -> None:
    if gate.stages and gate.stages[-1]["passed"]:
        detail = dict(gate.stages[-1]["detail"])
        detail["reason"] = failure.reason
        gate.stages[-1] = {**gate.stages[-1], "passed": False, "detail": detail}


def run_gate(script: str, args: argparse.Namespace, gate: Gate) -> None:
    {
        "s4_determinism_check": determinism_check,
        "s4_full_determinism_check": full_determinism_check,
        "s4_isolation_check": isolation_check,
        "s4_api_drift_check": api_drift_check,
    }[script](args, gate)


def determinism_check(args: argparse.Namespace, gate: Gate) -> None:
    replay_compare(args, gate, [0], "determinism_smoke")


def full_determinism_check(args: argparse.Namespace, gate: Gate) -> None:
    replay_compare(args, gate, [0, 1, 2, 3, 4], "full_determinism")


def replay_compare(
    args: argparse.Namespace,
    gate: Gate,
    default_seeds: list[int],
    comparison: str,
) -> None:
    seeds = parse_seed_list(args.seed_list, default_seeds)
    artifact_dirs = artifact_dirs_from_args(args)
    mode = replay_mode(artifact_dirs)

    gate.stage_start(1, "validate S4 D16 replay pins")
    input_detail = (
        dry_run_input_detail(args, artifact_dirs, seeds, mode)
        if args.dry_run
        else validate_s4_replay_inputs(args, artifact_dirs, seeds, mode)
    )
    gate.stage_done("d16_replay_inputs", 1, True, input_detail)

    first_artifact_dir = artifact_dirs[0] if artifact_dirs else None
    second_artifact_dir = artifact_dirs[1] if len(artifact_dirs) == 2 else first_artifact_dir

    gate.stage_start(2, "construct cold replay ledger 1")
    first_bytes, first_ledger = (None, None) if args.dry_run else replay_ledger(args, seeds, first_artifact_dir)
    first_detail = replay_detail(first_bytes, first_ledger, seeds, first_artifact_dir, mode)
    gate.stage_done("replay_1", 2, True, first_detail)
    emit_replay_run(gate, "run-1", first_detail)

    gate.stage_start(3, "construct cold replay ledger 2")
    second_bytes, second_ledger = (None, None) if args.dry_run else replay_ledger(args, seeds, second_artifact_dir)
    second_detail = replay_detail(second_bytes, second_ledger, seeds, second_artifact_dir, mode)
    gate.stage_done("replay_2", 3, True, second_detail)
    emit_replay_run(gate, "run-2", second_detail)

    gate.stage_start(4, "compare replay ledger bytes")
    matched = args.dry_run or first_bytes == second_bytes
    diff = {} if args.dry_run else replay_diff(first_ledger, second_ledger, first_bytes, second_bytes)
    detail = {
        "comparison": comparison,
        "mode": mode,
        "seed_list": seeds,
        "matched": matched,
        "first_sha": None if first_bytes is None else sha256_bytes(first_bytes),
        "second_sha": None if second_bytes is None else sha256_bytes(second_bytes),
        "diff": diff,
        "scope_limitations": scope_limitations(mode),
    }
    gate.stage_done("bytewise_compare", 4, matched, detail)
    gate.emit(
        {
            "event": "s4_determinism_compare",
            "run_a": "run-1",
            "run_b": "run-2",
            "matched": matched,
            "diff": diff,
        }
    )
    if not matched:
        raise GateFailure(4, "S4 replay evidence mismatch")


def parse_seed_list(raw: str | None, default: list[int]) -> list[int]:
    if raw is None:
        return default
    try:
        seeds = [int(part.strip()) for part in raw.split(",") if part.strip()]
    except ValueError as error:
        raise GateFailure(1, f"seed-list must be comma-separated integers: {raw!r}") from error
    if not seeds:
        raise GateFailure(1, "seed-list must contain at least one seed")
    if any(seed < 0 for seed in seeds):
        raise GateFailure(1, "seed-list cannot contain negative seeds")
    return seeds


def replay_mode(artifact_dirs: list[Path]) -> str:
    if len(artifact_dirs) >= 2:
        return "artifact_pair_replay"
    if artifact_dirs:
        return "single_artifact_tree_replay"
    return "fixture_pin_ledger_replay"


def dry_run_input_detail(
    args: argparse.Namespace,
    artifact_dirs: list[Path],
    seeds: list[int],
    mode: str,
) -> dict[str, Any]:
    return {
        "dry_run": True,
        "mode": mode,
        "seed_list": seeds,
        "gutenberg_fixture": rel(resolve(args.gutenberg_fixture)),
        "gutenberg_manifest": None
        if args.gutenberg_manifest is None
        else rel(resolve(args.gutenberg_manifest)),
        "artifact_dirs": [rel(path) for path in artifact_dirs],
        "build_kind": args.build_kind,
        "device_profile": args.device_profile,
        "scope_limitations": scope_limitations(mode),
    }


def validate_s4_replay_inputs(
    args: argparse.Namespace,
    artifact_dirs: list[Path],
    seeds: list[int],
    mode: str,
) -> dict[str, Any]:
    if len(artifact_dirs) > 2:
        raise GateFailure(1, "S4 determinism accepts at most two --artifact-dir values")
    if len(artifact_dirs) == 2:
        missing = [rel(path) for path in artifact_dirs if not path.exists()]
        if missing:
            raise GateFailure(1, f"two-replay artifact comparison requires existing roots: {missing}")
    fixture = load_gutenberg_fixture(resolve(args.gutenberg_fixture))
    manifest = load_gutenberg_manifest(args.gutenberg_manifest)
    return {
        "dry_run": False,
        "mode": mode,
        "seed_list": seeds,
        "gutenberg_fixture": fixture_summary(fixture),
        "gutenberg_manifest": manifest_summary(manifest),
        "artifact_dirs": [
            {
                "path": rel(path),
                "exists": path.exists(),
                "file_count": len(artifact_files(path)) if path.exists() else 0,
            }
            for path in artifact_dirs
        ],
        "build_kind": args.build_kind,
        "device_profile": args.device_profile,
        "scope_limitations": scope_limitations(mode),
    }


def replay_ledger(
    args: argparse.Namespace,
    seeds: list[int],
    artifact_dir: Path | None,
) -> tuple[bytes, dict[str, Any]]:
    fixture = load_gutenberg_fixture(resolve(args.gutenberg_fixture))
    manifest = load_gutenberg_manifest(args.gutenberg_manifest)
    artifact = artifact_snapshot(artifact_dir)
    ledger = {
        "schema": "s4_determinism_replay_ledger.v1",
        "evidence_source": "scripts/s4_determinism_check.sh",
        "seed_list": seeds,
        "build_kind": args.build_kind,
        "device_profile": args.device_profile,
        "pins": {
            "catalog_snapshot_sha256": first_present(
                manifest.get("catalog_snapshot_sha256"),
                fixture.get("catalog_snapshot_sha256"),
                artifact["pin_fields"].get("catalog_snapshot_sha256", [None])[0]
                if artifact["pin_fields"].get("catalog_snapshot_sha256")
                else None,
            ),
            "gutenberg_manifest_self_hash": first_present(
                manifest.get("manifest_self_hash"),
                first_pin(artifact, "gutenberg_manifest_self_hash"),
                first_pin(artifact, "manifest_self_hash"),
            ),
            "source_blob_sha256s": fixture["source_blob_sha256s"],
            "source_blob_list_sha": sha256_json(fixture["source_blob_sha256s"]),
            "build_kind": args.build_kind,
            "device_profile": args.device_profile,
        },
        "fixture": fixture,
        "gutenberg_manifest": manifest,
        "artifact_tree": artifact,
        "scope_limitations": scope_limitations("fixture_pin_ledger_replay" if artifact_dir is None else "artifact_pair_replay"),
    }
    return canonical_json_bytes(ledger), ledger


def replay_detail(
    payload: bytes | None,
    ledger: dict[str, Any] | None,
    seeds: list[int],
    artifact_dir: Path | None,
    mode: str,
) -> dict[str, Any]:
    if payload is None or ledger is None:
        return {
            "dry_run": True,
            "seed_list": seeds,
            "mode": mode,
            "artifact_dir": None if artifact_dir is None else rel(artifact_dir),
            "schema": None,
        }
    return {
        "dry_run": False,
        "seed_list": seeds,
        "mode": mode,
        "artifact_dir": None if artifact_dir is None else rel(artifact_dir),
        "schema": ledger["schema"],
        "payload_sha": sha256_bytes(payload),
        "hashes": pin_summary(ledger),
        "artifact_file_count": ledger["artifact_tree"]["file_count"],
        "artifact_seed_set": ledger["artifact_tree"]["seed_set"],
        "checked_source_blob_count": ledger["fixture"]["checked_source_blob_count"],
    }


def emit_replay_run(gate: Gate, run_id: str, detail: dict[str, Any]) -> None:
    gate.emit(
        {
            "event": "s4_determinism_run",
            "run_id": run_id,
            "hashes": detail.get("hashes", {}),
            "payload_sha": detail.get("payload_sha"),
            "artifact_file_count": detail.get("artifact_file_count"),
        }
    )


def replay_diff(
    first: dict[str, Any] | None,
    second: dict[str, Any] | None,
    first_bytes: bytes | None,
    second_bytes: bytes | None,
) -> dict[str, Any]:
    if first_bytes == second_bytes:
        return {}
    if first is None or second is None:
        return {"reason": "missing replay ledger"}
    first_files = file_sha_map(first["artifact_tree"])
    second_files = file_sha_map(second["artifact_tree"])
    missing = sorted(set(first_files).difference(second_files))
    extra = sorted(set(second_files).difference(first_files))
    changed = sorted(
        path for path in set(first_files).intersection(second_files) if first_files[path] != second_files[path]
    )
    first_pins = pin_summary(first)
    second_pins = pin_summary(second)
    pin_changes = sorted(key for key in set(first_pins).union(second_pins) if first_pins.get(key) != second_pins.get(key))
    return {
        "first_sha": None if first_bytes is None else sha256_bytes(first_bytes),
        "second_sha": None if second_bytes is None else sha256_bytes(second_bytes),
        "pin_changes": pin_changes,
        "artifact_missing": missing[:10],
        "artifact_extra": extra[:10],
        "artifact_changed": changed[:10],
        "artifact_changed_count": len(changed),
        "artifact_missing_count": len(missing),
        "artifact_extra_count": len(extra),
    }


def scope_limitations(mode: str) -> list[str]:
    limitations = [
        "S4 training/checkpoint replay is deferred until its producer bead lands; this gate compares the deterministic fixture pin ledger now.",
    ]
    if mode == "artifact_pair_replay":
        limitations.append("Supplied replay artifact trees are compared by relative path and exact payload hash.")
    elif mode == "single_artifact_tree_replay":
        limitations.append("One supplied artifact tree is included in both ledgers; pass means it is byte-stable as an input tree, not an independent cold replay pair.")
    else:
        limitations.append("No S4 artifact tree was supplied; pass covers the configured Gutenberg fixture pins and D16 identity fields only.")
    return limitations


def load_gutenberg_fixture(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise GateFailure(1, f"missing Gutenberg fixture pin file: {rel(path)}")
    raw = path.read_bytes()
    data = load_fixture_toml(raw.decode("utf-8"))
    sources = data.get("sources")
    if not isinstance(sources, list) or not sources:
        raise GateFailure(1, f"{rel(path)} must contain at least one [[sources]] row")
    book_ids = [source.get("book_id") for source in sources]
    if any(not isinstance(book_id, int) for book_id in book_ids):
        raise GateFailure(1, f"{rel(path)} source book_id values must be integers")
    if book_ids != sorted(book_ids):
        raise GateFailure(1, f"{rel(path)} source book_id values must be sorted")

    source_hashes = []
    payload_hashes = []
    missing_paths = []
    total_checked_bytes = 0
    schema = data.get("schema")
    for source in sources:
        book_id = source["book_id"]
        expected = normalize_hash(source.get("source_blob_sha256"), f"source_blob_sha256 for book {book_id}")
        source_hashes.append(expected)
        local_blob_path = source.get("local_blob_path")
        if not local_blob_path:
            continue
        blob_path = resolve(local_blob_path)
        if not blob_path.exists():
            missing_paths.append(local_blob_path)
            continue
        blob = blob_path.read_bytes()
        observed = sha256_bytes(blob)
        if observed != expected:
            raise GateFailure(
                1,
                f"source blob hash mismatch for book {book_id}: expected {expected}, observed {observed}",
            )
        declared_size = source.get("source_blob_size_bytes")
        if declared_size is not None and declared_size != len(blob):
            raise GateFailure(
                1,
                f"source blob byte length mismatch for book {book_id}: expected {declared_size}, observed {len(blob)}",
            )
        payload_hashes.append({"book_id": book_id, "sha256": observed, "bytes": len(blob)})
        total_checked_bytes += len(blob)
    if schema == "gutenberg_smoke_fixture.v1" and missing_paths:
        raise GateFailure(1, f"smoke fixture source blobs are missing: {missing_paths[:5]}")

    catalog = data.get("catalog_snapshot") if isinstance(data.get("catalog_snapshot"), dict) else {}
    catalog_sha = catalog.get("sha256") if catalog else None
    return {
        "path": rel(path),
        "schema": schema,
        "fixture_manifest_sha256": sha256_bytes(raw),
        "catalog_snapshot_sha256": None if catalog_sha is None else normalize_hash(catalog_sha, "catalog_snapshot.sha256"),
        "book_ids": book_ids,
        "book_count": len(book_ids),
        "source_blob_sha256s": source_hashes,
        "source_blob_list_sha": sha256_json(source_hashes),
        "source_blob_payloads": payload_hashes,
        "checked_source_blob_count": len(payload_hashes),
        "checked_source_blob_total_bytes": total_checked_bytes,
        "missing_source_blob_paths": missing_paths,
    }


def load_fixture_toml(text: str) -> dict[str, Any]:
    if tomllib is not None:
        return tomllib.loads(text)
    data: dict[str, Any] = {}
    current: dict[str, Any] = data
    for raw_line in text.splitlines():
        line = strip_toml_comment(raw_line).strip()
        if not line:
            continue
        if line == "[[sources]]":
            current = {}
            data.setdefault("sources", []).append(current)
            continue
        if line.startswith("[") and line.endswith("]"):
            name = line.strip("[]").strip()
            current = data.setdefault(name, {})
            continue
        if "=" not in line:
            continue
        key, raw_value = line.split("=", 1)
        current[key.strip()] = parse_toml_scalar(raw_value.strip())
    return data


def strip_toml_comment(line: str) -> str:
    in_string = False
    escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and in_string:
            escaped = True
            continue
        if char == '"':
            in_string = not in_string
            continue
        if char == "#" and not in_string:
            return line[:index]
    return line


def parse_toml_scalar(raw: str) -> Any:
    if raw.startswith('"') and raw.endswith('"'):
        return json.loads(raw)
    if raw in {"true", "false"}:
        return raw == "true"
    if re.fullmatch(r"[-+]?\d+", raw):
        return int(raw)
    if re.fullmatch(r"[-+]?\d+\.\d+", raw):
        return float(raw)
    return raw


def load_gutenberg_manifest(raw_path: str | None) -> dict[str, Any]:
    if raw_path is None:
        return {
            "path": None,
            "status": "not_supplied",
            "schema": None,
            "manifest_self_hash": None,
            "catalog_snapshot_sha256": None,
            "source_blob_sha256s": [],
        }
    path = resolve(raw_path)
    if not path.exists():
        raise GateFailure(1, f"missing Gutenberg manifest artifact: {rel(path)}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise GateFailure(1, f"Gutenberg manifest must be JSON: {rel(path)}: {error}") from error
    sources = data.get("sources", [])
    source_hashes = []
    if isinstance(sources, list):
        for index, source in enumerate(sources):
            if isinstance(source, dict) and source.get("source_blob_sha256") is not None:
                source_hashes.append(normalize_hash(source["source_blob_sha256"], f"manifest sources[{index}].source_blob_sha256"))
    manifest_hash = data.get("manifest_self_hash")
    catalog_hash = data.get("catalog_snapshot_sha256")
    return {
        "path": rel(path),
        "status": "supplied",
        "schema": data.get("schema"),
        "manifest_self_hash": None if manifest_hash is None else normalize_hash(manifest_hash, "manifest_self_hash"),
        "catalog_snapshot_sha256": None if catalog_hash is None else normalize_hash(catalog_hash, "catalog_snapshot_sha256"),
        "source_blob_sha256s": source_hashes,
        "source_blob_list_sha": sha256_json(source_hashes),
        "payload_sha": sha256_bytes(path.read_bytes()),
    }


def artifact_snapshot(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {
            "kind": "none",
            "file_count": 0,
            "files": [],
            "pin_fields": {},
            "result_self_hashes": {},
            "seed_set": [],
        }
    files = []
    pin_fields: dict[str, set[str]] = {}
    result_hashes: dict[str, set[str]] = {}
    for candidate in artifact_files(path):
        blob = candidate.read_bytes()
        relative = candidate.name if path.is_file() else str(candidate.relative_to(path))
        fields = extract_artifact_fields(blob)
        for key, values in fields.items():
            target = result_hashes if key in RESULT_SELF_HASH_FIELDS else pin_fields
            target.setdefault(key, set()).update(values)
        files.append(
            {
                "path": relative,
                "bytes": len(blob),
                "sha256": sha256_bytes(blob),
                "seed": seed_from_path(candidate),
            }
        )
    return {
        "kind": "file" if path.is_file() else "directory",
        "exists": path.exists(),
        "file_count": len(files),
        "files": sorted(files, key=lambda row: row["path"]),
        "pin_fields": sorted_field_map(pin_fields),
        "result_self_hashes": sorted_field_map(result_hashes),
        "seed_set": sorted({row["seed"] for row in files if row["seed"] is not None}),
    }


def artifact_files(path: Path) -> list[Path]:
    if path.is_file():
        return [path]
    if not path.exists():
        return []
    return sorted(candidate for candidate in path.rglob("*") if candidate.is_file())


def extract_artifact_fields(blob: bytes) -> dict[str, list[str]]:
    fields: dict[str, set[str]] = {}
    try:
        payload = json.loads(blob.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        payload = None
    if payload is not None:
        collect_json_fields(payload, fields)
    text = blob[:2_000_000].decode("utf-8", errors="ignore")
    for key in PIN_FIELDS.union(RESULT_SELF_HASH_FIELDS):
        pattern = re.compile(rf"{re.escape(key)}\s*[:=]\s*\"([^\"]+)\"")
        for match in pattern.finditer(text):
            fields.setdefault(key, set()).add(normalize_field_value(key, match.group(1)))
    return sorted_field_map(fields)


def collect_json_fields(value: Any, fields: dict[str, set[str]]) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            if key in PIN_FIELDS.union(RESULT_SELF_HASH_FIELDS):
                for normalized in normalize_json_field_values(key, item):
                    fields.setdefault(key, set()).add(normalized)
            collect_json_fields(item, fields)
    elif isinstance(value, list):
        for item in value:
            collect_json_fields(item, fields)


def normalize_json_field_values(key: str, value: Any) -> list[str]:
    if isinstance(value, str):
        return [normalize_field_value(key, value)]
    if isinstance(value, list):
        values = []
        for item in value:
            values.extend(normalize_json_field_values(key, item))
        return values
    return []


def normalize_field_value(key: str, value: str) -> str:
    if key.endswith("sha256") or key.endswith("_hash"):
        return normalize_hash(value, key)
    return value


def normalize_hash(value: Any, field: str) -> str:
    if not isinstance(value, str):
        raise GateFailure(1, f"{field} must be a sha256 string")
    if HASH_RE.fullmatch(value):
        return value
    if RAW_HASH_RE.fullmatch(value):
        return f"sha256:{value}"
    raise GateFailure(1, f"{field} must be sha256:<64 lowercase hex>")


def sorted_field_map(values: dict[str, set[str]]) -> dict[str, list[str]]:
    return {key: sorted(items) for key, items in sorted(values.items())}


def fixture_summary(fixture: dict[str, Any]) -> dict[str, Any]:
    return {
        "path": fixture["path"],
        "schema": fixture["schema"],
        "fixture_manifest_sha256": fixture["fixture_manifest_sha256"],
        "catalog_snapshot_sha256": fixture["catalog_snapshot_sha256"],
        "book_count": fixture["book_count"],
        "source_blob_count": len(fixture["source_blob_sha256s"]),
        "source_blob_list_sha": fixture["source_blob_list_sha"],
        "checked_source_blob_count": fixture["checked_source_blob_count"],
        "checked_source_blob_total_bytes": fixture["checked_source_blob_total_bytes"],
        "missing_source_blob_count": len(fixture["missing_source_blob_paths"]),
    }


def manifest_summary(manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        "path": manifest["path"],
        "status": manifest["status"],
        "schema": manifest["schema"],
        "manifest_self_hash": manifest["manifest_self_hash"],
        "catalog_snapshot_sha256": manifest["catalog_snapshot_sha256"],
        "source_blob_count": len(manifest["source_blob_sha256s"]),
        "source_blob_list_sha": manifest.get("source_blob_list_sha"),
    }


def pin_summary(ledger: dict[str, Any]) -> dict[str, Any]:
    pins = ledger["pins"]
    return {
        "catalog_snapshot_sha256": pins["catalog_snapshot_sha256"],
        "gutenberg_manifest_self_hash": pins["gutenberg_manifest_self_hash"],
        "source_blob_count": len(pins["source_blob_sha256s"]),
        "source_blob_list_sha": pins["source_blob_list_sha"],
        "build_kind": pins["build_kind"],
        "device_profile": pins["device_profile"],
    }


def file_sha_map(artifact_tree: dict[str, Any]) -> dict[str, str]:
    return {row["path"]: row["sha256"] for row in artifact_tree["files"]}


def first_pin(artifact: dict[str, Any], key: str) -> str | None:
    values = artifact["pin_fields"].get(key) or artifact["result_self_hashes"].get(key)
    return values[0] if values else None


def first_present(*values: str | None) -> str | None:
    return next((value for value in values if value), None)


def sha256_json(value: Any) -> str:
    return sha256_bytes(canonical_json_bytes(value))


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def isolation_check(args: argparse.Namespace, gate: Gate) -> None:
    rng_path = ROOT / "gbf-experiments" / "src" / "s4" / "rng.rs"
    gate.stage_start(1, "validate S4 RNG module exists")
    gate.stage_done("rng_module", 1, rng_path.exists(), {"path": rel(rng_path), "exists": rng_path.exists()})
    if not rng_path.exists():
        raise GateFailure(1, "missing gbf-experiments::s4::rng module")

    source = rng_path.read_text(encoding="utf-8")
    gate.stage_start(2, "assert S4 training RNG domains use s4-* convention")
    domains = sorted(set(re.findall(r'"(s4-[^"]+)"', source)))
    passed = bool(domains) and all(domain.startswith("s4-") for domain in domains)
    gate.stage_done("s4_rng_domain_prefix", 2, passed, {"domains": domains})
    if not passed:
        raise GateFailure(2, "S4 RNG domains must use s4-* strings")

    gate.stage_start(3, "assert no S1/S3 RNG domain reuse")
    reused = [domain for domain in domains if domain.startswith(("s1-", "s2-", "s3-"))]
    gate.stage_done("cross_slice_rng_domain_reuse", 3, not reused, {"reused": reused})
    if reused:
        raise GateFailure(3, "S4 RNG domains reuse an earlier slice prefix")


def api_drift_check(args: argparse.Namespace, gate: Gate) -> None:
    mod_path = ROOT / "gbf-experiments" / "src" / "s4" / "mod.rs"
    gate.stage_start(1, "validate S4 public module surface exists")
    gate.stage_done("s4_module_input", 1, mod_path.exists(), {"path": rel(mod_path), "exists": mod_path.exists()})
    if not mod_path.exists():
        raise GateFailure(1, "missing gbf-experiments::s4 module")

    observed = public_module_surface(mod_path)
    missing = [item for item in EXPECTED_S4_MODULE_SURFACE if item not in observed]
    optional = [item for item in observed if item in OPTIONAL_S4_MODULE_SURFACE]
    expected_or_optional = set(EXPECTED_S4_MODULE_SURFACE).union(OPTIONAL_S4_MODULE_SURFACE)
    extra = [item for item in observed if item not in expected_or_optional]
    passed = not missing and (args.dry_run or not extra)
    gate.stage_start(2, "compare S4 public module surface")
    gate.stage_done(
        "s4_public_module_surface",
        2,
        passed,
        {
            "missing": missing,
            "extra": extra,
            "optional": {item: OPTIONAL_S4_MODULE_SURFACE[item] for item in optional},
            "observed_count": len(observed),
        },
    )
    if not passed:
        raise GateFailure(2, "S4 public module surface drift detected")


def artifact_dirs_from_args(args: argparse.Namespace) -> list[Path]:
    return [resolve(path) for path in (args.artifact_dir or [])]


def discover_result_evidence(paths: list[Path]) -> list[dict[str, Any]]:
    rows = []
    for path in paths:
        candidates = [path] if path.is_file() else sorted(path.rglob("*")) if path.exists() else []
        for candidate in candidates:
            if not candidate.is_file():
                continue
            try:
                blob = candidate.read_bytes()
            except OSError:
                continue
            if not S4_HASH_RE.search(blob):
                continue
            rows.append({"path": rel(candidate), "seed": seed_from_path(candidate), "payload_sha": sha256_bytes(blob)})
    return rows


def seed_from_path(path: Path) -> int | None:
    match = SEED_RE.search(str(path))
    if not match:
        return None
    value = next(group for group in match.groups() if group is not None)
    return int(value)


def public_module_surface(path: Path) -> list[str]:
    lines = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip().rstrip(";")
        if stripped.startswith("pub mod "):
            lines.append(stripped)
    return sorted(lines)


def resolve(path: str | Path) -> Path:
    candidate = Path(path)
    if candidate.is_absolute():
        return candidate
    return ROOT / candidate


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def sha256_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
