#!/usr/bin/env python3
"""Audit F-S7 bead-level Gemini/Claude review coverage."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


REVIEWERS = ("gemini", "claude")
DEFAULT_LABELS = ("slice:S7", "s7")
TOMBSTONE_STATUSES = {"tombstone"}
CLOSED_STATUSES = {"closed"}
MANAGER_DISPOSITION_RE = re.compile(
    r"manager disposition:.*no additional acpx review required",
    re.IGNORECASE | re.DOTALL,
)
NEGATIVE_PASS_CONTEXT_RE = re.compile(
    r"\b(no|without|missing|lacks?|failed|fails?)\b.{0,120}\bpass\b"
    r"|\bpass\b.{0,120}\b(not|never)\s+(claimed|written|recorded|available)\b",
    re.IGNORECASE,
)
NEGATIVE_REVIEW_TERMS = (
    "needs_changes",
    "needs changes",
    "failed before review",
    "fails before review",
    "verdict: concerns",
    "verdict: blockers",
    "verdict: needs_changes",
)
NON_BLOCKING_REVIEW_RE = re.compile(
    r"\b(concerns?|caveats?)\b.{0,160}\b(non-blocking|not code blockers?|not blockers?)\b"
    r"|\b(non-blocking|not code blockers?|not blockers?)\b.{0,160}\b(concerns?|caveats?)\b",
    re.IGNORECASE,
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Audit non-tombstone F-S7 beads for Gemini and Claude ACPX review "
            "evidence or an explicit manager disposition."
        )
    )
    parser.add_argument("--root", default=".", help="repository root containing the beads DB")
    parser.add_argument(
        "--issues-file",
        help=(
            "JSON fixture containing full issue objects; when omitted the audit "
            "loads issues from br labels slice:S7 and s7"
        ),
    )
    parser.add_argument(
        "--label",
        action="append",
        dest="labels",
        help="beads label to include when loading through br; may be repeated",
    )
    parser.add_argument(
        "--include-tombstones",
        action="store_true",
        help="audit tombstone records instead of reporting them as skipped",
    )
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    args = parser.parse_args()

    try:
        issues = load_issues(args)
    except AuditInputError as error:
        if args.json:
            print(json.dumps({"status": "error", "errors": [str(error)]}, indent=2))
        else:
            print("S7 bead review coverage: NEEDS_CHANGES")
            print(f" - {error}")
        return 1

    rows, errors = audit_issues(issues, include_tombstones=args.include_tombstones)
    if args.json:
        print(
            json.dumps(
                {
                    "status": "ok" if not errors else "needs_changes",
                    "issue_count": len(rows),
                    "errors": errors,
                    "issues": rows,
                },
                indent=2,
                sort_keys=True,
            )
        )
    elif errors:
        print("S7 bead review coverage: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
    else:
        print(f"S7 bead review coverage: ok ({len(rows)} audited issue records)")
    return 1 if errors else 0


class AuditInputError(RuntimeError):
    """Raised when audit input cannot be loaded."""


def load_issues(args: argparse.Namespace) -> list[dict[str, Any]]:
    if args.issues_file:
        payload = read_json(Path(args.issues_file))
        issues = coerce_issue_list(payload, str(args.issues_file))
        return [issue for issue in issues if is_issue_object(issue)]
    labels = tuple(args.labels or DEFAULT_LABELS)
    return load_issues_from_br(Path(args.root), labels)


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise AuditInputError(f"could not read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise AuditInputError(f"{path} is not valid JSON: {error}") from error


def coerce_issue_list(payload: Any, source: str) -> list[dict[str, Any]]:
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        for key in ("issues", "items", "records"):
            value = payload.get(key)
            if isinstance(value, list):
                return value
    raise AuditInputError(f"{source} must contain an issue list")


def is_issue_object(value: Any) -> bool:
    return isinstance(value, dict) and isinstance(value.get("id"), str)


def load_issues_from_br(root: Path, labels: tuple[str, ...]) -> list[dict[str, Any]]:
    ids: set[str] = set()
    for label in labels:
        listing = run_br_json(root, "list", "--all", "--label", label, "--json")
        for issue in coerce_issue_list(listing, f"br list --label {label}"):
            issue_id = issue.get("id")
            if isinstance(issue_id, str):
                ids.add(issue_id)

    if not ids:
        raise AuditInputError(f"no F-S7 bead records found for labels: {', '.join(labels)}")

    issues: list[dict[str, Any]] = []
    for issue_id in sorted(ids):
        detail = run_br_json(root, "show", issue_id, "--json")
        detail_issues = coerce_issue_list(detail, f"br show {issue_id}")
        if not detail_issues:
            raise AuditInputError(f"br show {issue_id} returned no issue records")
        issue = detail_issues[0]
        if is_issue_object(issue):
            issues.append(issue)
    return issues


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


def audit_issues(
    issues: list[dict[str, Any]], *, include_tombstones: bool
) -> tuple[list[dict[str, Any]], list[str]]:
    rows: list[dict[str, Any]] = []
    errors: list[str] = []

    for issue in sorted(issues, key=lambda item: str(item.get("id", ""))):
        issue_id = str(issue.get("id", ""))
        status = str(issue.get("status", "")).strip().lower()
        if status in TOMBSTONE_STATUSES and not include_tombstones:
            rows.append(
                {
                    "id": issue_id,
                    "status": status,
                    "reviewers": {},
                    "audit_status": "skipped_tombstone",
                }
            )
            continue

        text = issue_text(issue)
        reviewer_state = {reviewer: reviewer_coverage(text, reviewer) for reviewer in REVIEWERS}
        row_errors: list[str] = []
        if status not in CLOSED_STATUSES:
            row_errors.append(f"status is {status or 'unknown'}")
        missing = [
            reviewer
            for reviewer, state in reviewer_state.items()
            if state not in {"pass", "manager_disposition", "non_blocking_review"}
        ]
        if missing:
            row_errors.append(f"missing {', '.join(missing)} review evidence")

        audit_status = "ok" if not row_errors else "needs_changes"
        rows.append(
            {
                "id": issue_id,
                "status": status,
                "reviewers": reviewer_state,
                "audit_status": audit_status,
            }
        )
        if row_errors:
            errors.append(f"{issue_id}: {'; '.join(row_errors)}")

    if not rows:
        errors.append("no issue records were audited")
    return rows, errors


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


def reviewer_coverage(text: str, reviewer: str) -> str:
    if has_reviewer_pass(text, reviewer):
        return "pass"
    if has_non_blocking_review(text, reviewer):
        return "non_blocking_review"
    if has_manager_disposition(text):
        return "manager_disposition"
    return "missing"


def has_reviewer_pass(text: str, reviewer: str) -> bool:
    for segment in evidence_segments(text):
        lowered = segment.lower()
        if reviewer not in lowered or "pass" not in lowered:
            continue
        if NEGATIVE_PASS_CONTEXT_RE.search(segment):
            continue
        if any(term in lowered for term in NEGATIVE_REVIEW_TERMS):
            continue
        return True
    return False


def has_non_blocking_review(text: str, reviewer: str) -> bool:
    for segment in evidence_segments(text):
        lowered = segment.lower()
        if reviewer not in lowered:
            continue
        if any(term in lowered for term in ("needs_changes", "failed before review")):
            continue
        if NON_BLOCKING_REVIEW_RE.search(segment):
            return True
    return False


def evidence_segments(text: str) -> list[str]:
    segments = [line.strip() for line in text.splitlines() if line.strip()]
    segments.extend(part.strip() for part in re.split(r"\n\s*\n", text) if part.strip())
    return segments


def has_manager_disposition(text: str) -> bool:
    return bool(MANAGER_DISPOSITION_RE.search(text))


if __name__ == "__main__":
    sys.exit(main())
