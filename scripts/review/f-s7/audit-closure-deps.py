#!/usr/bin/env python3
"""Audit bd-2v9r closure dependencies before final F-S7 packet closeout."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


DEFAULT_ISSUE = "bd-2v9r"
BLOCKING_DEP_TYPES = {"blocks", "parent-child"}
RESOLVED_STATUSES = {"closed", "tombstone"}


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Audit direct bd-2v9r dependency edges that must be resolved or "
            "explicitly dispositioned before final F-S7 closure."
        )
    )
    parser.add_argument("--root", default=".", help="repository root containing the beads DB")
    parser.add_argument("--issue", default=DEFAULT_ISSUE, help="closure issue id to audit")
    parser.add_argument(
        "--issue-file",
        help="JSON fixture containing one issue object; when omitted the audit loads br show <issue>",
    )
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    args = parser.parse_args()

    try:
        issue = load_issue(args)
    except AuditInputError as error:
        return emit(args.json, [], [str(error)])

    rows, errors = audit_issue(issue)
    return emit(args.json, rows, errors)


class AuditInputError(RuntimeError):
    """Raised when audit input cannot be loaded."""


def load_issue(args: argparse.Namespace) -> dict[str, Any]:
    if args.issue_file:
        payload = read_json(Path(args.issue_file))
        return coerce_issue(payload, str(args.issue_file))
    payload = run_br_json(Path(args.root), "show", args.issue, "--json")
    return coerce_issue(payload, f"br show {args.issue}")


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise AuditInputError(f"could not read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise AuditInputError(f"{path} is not valid JSON: {error}") from error


def coerce_issue(payload: Any, source: str) -> dict[str, Any]:
    if isinstance(payload, list) and payload:
        payload = payload[0]
    if isinstance(payload, dict) and isinstance(payload.get("id"), str):
        return payload
    raise AuditInputError(f"{source} must contain an issue object")


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


def audit_issue(issue: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    issue_id = str(issue.get("id", ""))
    rows: list[dict[str, Any]] = []
    errors: list[str] = []
    for dependency in sorted(issue.get("dependencies", []), key=dependency_sort_key):
        if not isinstance(dependency, dict):
            continue
        dep_type = str(dependency.get("dep_type", "")).strip().lower()
        if dep_type not in BLOCKING_DEP_TYPES:
            continue
        dep_id = str(dependency.get("id", "")).strip()
        status = str(dependency.get("status", "")).strip().lower()
        title = str(dependency.get("title", "")).strip()
        dispositioned = has_dependency_disposition(issue, dep_id)
        audit_status = "ok" if status in RESOLVED_STATUSES or dispositioned else "needs_changes"
        rows.append(
            {
                "id": dep_id,
                "status": status,
                "dep_type": dep_type,
                "title": title,
                "dispositioned": dispositioned,
                "audit_status": audit_status,
            }
        )
        if audit_status != "ok":
            errors.append(
                f"{issue_id}: unresolved dependency {dep_id} "
                f"status={status or 'unknown'} type={dep_type} title={title!r}"
            )
    return rows, errors


def dependency_sort_key(dependency: Any) -> tuple[str, str]:
    if not isinstance(dependency, dict):
        return ("", "")
    return (str(dependency.get("dep_type", "")), str(dependency.get("id", "")))


def has_dependency_disposition(issue: dict[str, Any], dep_id: str) -> bool:
    if not dep_id:
        return False
    pattern = re.compile(
        rf"s7 closure dependency disposition:\s*{re.escape(dep_id)}\b.*\bnon[- ]blocking\b",
        re.IGNORECASE | re.DOTALL,
    )
    return bool(pattern.search(issue_text(issue)))


def issue_text(issue: dict[str, Any]) -> str:
    parts: list[str] = []
    for field in ("id", "title", "description", "notes", "close_reason"):
        value = issue.get(field)
        if isinstance(value, str):
            parts.append(value)
    for comment in issue.get("comments", []):
        if isinstance(comment, dict) and isinstance(comment.get("text"), str):
            parts.append(comment["text"])
    return "\n".join(parts)


def emit(as_json: bool, rows: list[dict[str, Any]], errors: list[str]) -> int:
    if as_json:
        print(
            json.dumps(
                {
                    "status": "ok" if not errors else "needs_changes",
                    "dependency_count": len(rows),
                    "errors": errors,
                    "dependencies": rows,
                },
                indent=2,
                sort_keys=True,
            )
        )
    elif errors:
        print("S7 closure dependency audit: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
    else:
        print(f"S7 closure dependency audit: ok ({len(rows)} blocking dependency edges audited)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
