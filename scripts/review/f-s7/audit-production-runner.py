#!/usr/bin/env python3
"""Audit the real F-S7 production-runner owner before closure."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


DEFAULT_CLOSURE_ISSUE = "bd-2v9r"
DEFAULT_RUNNER_ISSUE = "bd-3e10j"
BLOCKING_DEP_TYPES = {"blocks", "parent-child"}
RESOLVED_STATUSES = {"closed", "tombstone"}
REQUIRED_CONTRACT_SNIPPETS = (
    "s7_production_bundle_manifest.v1",
    "MoeTinyDenseMatched",
    "Gutenberg",
    "optimizer/model-state",
    "s7_run_log.v1",
    "s7_grad_log.v1",
    "s7_router_step_telemetry.v1",
    "validate-artifacts.py",
    "production_closure_retrain_score",
)
PRODUCTION_ARTIFACT_GLOBS = (
    "experiments/S7/runs/*/seed-*/run-log.json",
    "experiments/S7/runs/*/seed-*/grad-log.jsonl",
    "experiments/S7/runs/*/seed-*/router-step-telemetry.jsonl",
    "experiments/S7/scores/*/seed-*/score.json",
    "experiments/S7/switch-stats/seed-*/switch-stats.json",
    "experiments/S7/router-collapse/seed-0/sweep.json",
    "experiments/S7/dense-vs-moe/comparison.json",
    "experiments/S7/frontier/frontier.json",
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Verify that the real F-S7 production training runner is an owned, "
            "resolved closure dependency before bd-2v9r can consume production artifacts."
        )
    )
    parser.add_argument("--root", default=".", help="repository root containing the beads DB")
    parser.add_argument("--closure-issue", default=DEFAULT_CLOSURE_ISSUE)
    parser.add_argument("--runner-issue", default=DEFAULT_RUNNER_ISSUE)
    parser.add_argument("--closure-issue-file", help="JSON fixture for the closure issue")
    parser.add_argument("--runner-issue-file", help="JSON fixture for the runner issue")
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    args = parser.parse_args()

    try:
        closure_issue = load_issue(
            Path(args.root),
            args.closure_issue,
            args.closure_issue_file,
            "closure issue",
        )
        runner_issue = load_issue(
            Path(args.root),
            args.runner_issue,
            args.runner_issue_file,
            "production-runner issue",
        )
    except AuditInputError as error:
        return emit(args.json, {"status": "needs_changes"}, [str(error)])

    result, errors = audit(
        Path(args.root),
        closure_issue,
        runner_issue,
        args.runner_issue,
    )
    return emit(args.json, result, errors)


class AuditInputError(RuntimeError):
    """Raised when audit input cannot be loaded."""


def load_issue(root: Path, issue_id: str, issue_file: str | None, label: str) -> dict[str, Any]:
    if issue_file:
        payload = read_json(Path(issue_file))
        return coerce_issue(payload, issue_file, label)
    payload = run_br_json(root, "show", issue_id, "--json")
    return coerce_issue(payload, f"br show {issue_id}", label)


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise AuditInputError(f"could not read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise AuditInputError(f"{path} is not valid JSON: {error}") from error


def coerce_issue(payload: Any, source: str, label: str) -> dict[str, Any]:
    if isinstance(payload, list) and payload:
        payload = payload[0]
    if isinstance(payload, dict) and isinstance(payload.get("id"), str):
        return payload
    raise AuditInputError(f"{source} must contain a {label} object")


def run_br_json(root: Path, *args: str) -> Any:
    command = ("br",) + args
    proc = subprocess.run(
        command,
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip()
        raise AuditInputError(f"{' '.join(command)} failed{': ' + detail if detail else ''}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as error:
        raise AuditInputError(f"{' '.join(command)} did not emit valid JSON: {error}") from error


def audit(
    root: Path,
    closure_issue: dict[str, Any],
    runner_issue: dict[str, Any],
    expected_runner_id: str,
) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    closure_id = str(closure_issue.get("id", ""))
    runner_id = str(runner_issue.get("id", ""))
    runner_status = str(runner_issue.get("status", "")).strip().lower()
    runner_resolved = runner_status in RESOLVED_STATUSES
    dependency = find_runner_dependency(closure_issue, expected_runner_id)
    production_artifacts = find_existing_production_artifacts(root)

    if runner_id != expected_runner_id:
        errors.append(
            f"production-runner issue mismatch: expected {expected_runner_id}, observed {runner_id or 'missing'}"
        )
    if dependency is None:
        errors.append(
            f"{closure_id or DEFAULT_CLOSURE_ISSUE}: missing blocking dependency on {expected_runner_id}"
        )
    elif str(dependency.get("dep_type", "")).strip().lower() not in BLOCKING_DEP_TYPES:
        errors.append(
            f"{closure_id or DEFAULT_CLOSURE_ISSUE}: dependency {expected_runner_id} must be blocking, observed {dependency.get('dep_type')!r}"
        )
    if not runner_resolved:
        errors.append(
            f"{expected_runner_id}: production runner is {runner_status or 'unknown'}, not resolved; do not consume fixture/smoke/replay artifacts as production"
        )

    contract_text = issue_text(runner_issue)
    missing_snippets = [
        snippet for snippet in REQUIRED_CONTRACT_SNIPPETS if snippet not in contract_text
    ]
    if missing_snippets:
        errors.append(
            f"{expected_runner_id}: production-runner contract missing required phrase(s): {', '.join(missing_snippets)}"
        )

    if production_artifacts and not runner_resolved:
        preview = ", ".join(production_artifacts[:5])
        suffix = "" if len(production_artifacts) <= 5 else f", ... +{len(production_artifacts) - 5}"
        errors.append(
            f"{expected_runner_id}: production-looking S7 artifacts exist while runner owner is unresolved: {preview}{suffix}"
        )

    return (
        {
            "status": "ok" if not errors else "needs_changes",
            "closure_issue": closure_id,
            "runner_issue": runner_id,
            "runner_status": runner_status,
            "runner_resolved": runner_resolved,
            "blocking_dependency_present": dependency is not None,
            "production_artifact_count": len(production_artifacts),
        },
        errors,
    )


def find_runner_dependency(issue: dict[str, Any], runner_id: str) -> dict[str, Any] | None:
    for dependency in issue.get("dependencies", []):
        if not isinstance(dependency, dict):
            continue
        if str(dependency.get("id", "")).strip() == runner_id:
            return dependency
    return None


def find_existing_production_artifacts(root: Path) -> list[str]:
    paths: list[str] = []
    for pattern in PRODUCTION_ARTIFACT_GLOBS:
        for path in sorted(root.glob(pattern)):
            if path.is_file():
                paths.append(path.as_posix())
    return paths


def issue_text(issue: dict[str, Any]) -> str:
    parts: list[str] = []
    for field in ("id", "title", "description", "acceptance_criteria", "notes", "close_reason"):
        value = issue.get(field)
        if isinstance(value, str):
            parts.append(value)
    for comment in issue.get("comments", []):
        if isinstance(comment, dict) and isinstance(comment.get("text"), str):
            parts.append(comment["text"])
    return "\n".join(parts)


def emit(as_json: bool, result: dict[str, Any], errors: list[str]) -> int:
    if as_json:
        payload = dict(result)
        payload["errors"] = errors
        print(json.dumps(payload, indent=2, sort_keys=True))
    elif errors:
        print("S7 production runner audit: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
    else:
        print("S7 production runner audit: ok")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
