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
DEFAULT_EVIDENCE_DIR = "docs/review/f-s7/bead-reviews"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
ACPX_COMMAND_RE = re.compile(r"^\s*acpx(?:\s|$)", re.IGNORECASE)
ALWAYS_ON_PERSONAS = {"P5", "P6"}
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
    parser.add_argument(
        "--evidence-dir",
        default=DEFAULT_EVIDENCE_DIR,
        help="directory containing optional s7_bead_acpx_review.v1 JSON evidence",
    )
    parser.add_argument(
        "--expected-head",
        help="40-hex git commit that every structured review evidence file must name as reviewed_head",
    )
    parser.add_argument(
        "--allow-reviewed-head-ancestor-of",
        help=(
            "40-hex git commit that reviewed_head may equal or precede when only "
            "review evidence/admin files changed after the review"
        ),
    )
    parser.add_argument(
        "--require-reviewed-diff-admin-only",
        action="store_true",
        help=(
            "with --allow-reviewed-head-ancestor-of, require every file changed "
            "after reviewed_head to be review evidence or .beads/issues.jsonl"
        ),
    )
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    args = parser.parse_args()

    root = Path(args.root)
    try:
        issues = load_issues(args)
    except AuditInputError as error:
        if args.json:
            print(json.dumps({"status": "error", "errors": [str(error)]}, indent=2))
        else:
            print("S7 bead review coverage: NEEDS_CHANGES")
            print(f" - {error}")
        return 1

    rows, errors = audit_issues(
        issues,
        include_tombstones=args.include_tombstones,
        root=root,
        evidence_dir=(root / args.evidence_dir).resolve(),
        expected_head=args.expected_head,
        allow_ancestor_head=args.allow_reviewed_head_ancestor_of,
        require_admin_only_diff=args.require_reviewed_diff_admin_only,
    )
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
    issues: list[dict[str, Any]],
    *,
    include_tombstones: bool,
    root: Path,
    evidence_dir: Path | None,
    expected_head: str | None = None,
    allow_ancestor_head: str | None = None,
    require_admin_only_diff: bool = False,
) -> tuple[list[dict[str, Any]], list[str]]:
    rows: list[dict[str, Any]] = []
    errors: list[str] = []
    if expected_head is not None and not COMMIT_RE.match(expected_head):
        errors.append(f"expected_head must be a 40-hex commit id, observed {expected_head!r}")
    if allow_ancestor_head is not None and not COMMIT_RE.match(allow_ancestor_head):
        errors.append(
            f"allow_reviewed_head_ancestor_of must be a 40-hex commit id, observed {allow_ancestor_head!r}"
        )
    if require_admin_only_diff and allow_ancestor_head is None:
        errors.append(
            "--require-reviewed-diff-admin-only requires --allow-reviewed-head-ancestor-of"
        )

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
        reviewer_state: dict[str, str] = {}
        row_errors: list[str] = []
        for reviewer in REVIEWERS:
            state = reviewer_coverage(text, reviewer)
            if state == "missing" and evidence_dir is not None:
                state, evidence_error = structured_reviewer_coverage(
                    root,
                    evidence_dir,
                    issue_id,
                    reviewer,
                    expected_head,
                    allow_ancestor_head,
                    require_admin_only_diff,
                )
                if evidence_error is not None:
                    row_errors.append(evidence_error)
            reviewer_state[reviewer] = state
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


def structured_reviewer_coverage(
    root: Path,
    evidence_dir: Path,
    issue_id: str,
    reviewer: str,
    expected_head: str | None,
    allow_ancestor_head: str | None,
    require_admin_only_diff: bool,
) -> tuple[str, str | None]:
    path = evidence_dir / f"{issue_id}-{reviewer}.json"
    if not path.is_file():
        return "missing", None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        return "missing", f"{issue_id}: {path} is not valid JSON: {error}"
    if not isinstance(payload, dict):
        return "missing", f"{issue_id}: {path} must contain a JSON object"

    errors: list[str] = []
    expect_structured_equal(errors, path, payload, "schema", "s7_bead_acpx_review.v1")
    expect_structured_equal(errors, path, payload, "bead", issue_id)
    expect_structured_equal(errors, path, payload, "reviewer", reviewer)
    expect_structured_equal(errors, path, payload, "transport", "acpx")
    expect_structured_equal(errors, path, payload, "verdict", "PASS")
    if not ACPX_COMMAND_RE.search(str(payload.get("command", ""))):
        errors.append(f"{path} command must record an ACPX invocation prefix")
    reviewed_head = str(payload.get("reviewed_head", ""))
    if not COMMIT_RE.match(reviewed_head):
        errors.append(f"{path} reviewed_head must be a 40-hex commit id")
    elif expected_head is not None and reviewed_head != expected_head:
        errors.append(
            f"{path} reviewed_head must match expected_head {expected_head}, observed {reviewed_head}"
        )
    elif expected_head is None and allow_ancestor_head is not None:
        validate_reviewed_head_ancestry(
            errors,
            root,
            path,
            evidence_dir,
            reviewed_head,
            allow_ancestor_head,
            require_admin_only_diff,
        )
    if not non_empty_string(payload.get("summary")):
        errors.append(f"{path} summary must be a non-empty string")
    personas = payload.get("personas")
    if not isinstance(personas, list) or not all(isinstance(item, str) for item in personas):
        errors.append(f"{path} personas must be a list of persona ids")
        observed_personas: set[str] = set()
    else:
        observed_personas = set(personas)
    missing_always_on = sorted(ALWAYS_ON_PERSONAS - observed_personas)
    if missing_always_on:
        errors.append(f"{path} missing always-on persona(s): {missing_always_on}")
    findings = payload.get("findings")
    if not isinstance(findings, list):
        errors.append(f"{path} findings must be a list")
    if errors:
        return "missing", f"{issue_id}: {'; '.join(errors)}"
    return "pass", None


def validate_reviewed_head_ancestry(
    errors: list[str],
    root: Path,
    path: Path,
    evidence_dir: Path,
    reviewed_head: str,
    current_head: str,
    require_admin_only_diff: bool,
) -> None:
    ancestor = run_git(root, "merge-base", "--is-ancestor", reviewed_head, current_head)
    if ancestor.returncode != 0:
        errors.append(
            f"{path} reviewed_head must be current head or an ancestor of {current_head}, observed {reviewed_head}"
        )
        return
    if not require_admin_only_diff or reviewed_head == current_head:
        return

    changed = run_git(root, "diff", "--name-only", f"{reviewed_head}..{current_head}", "--")
    if changed.returncode != 0:
        detail = (changed.stderr or changed.stdout).strip()
        errors.append(f"{path} could not inspect post-review diff: {detail}")
        return

    unexpected = [
        item.strip()
        for item in changed.stdout.splitlines()
        if item.strip() and not is_review_admin_path(root, evidence_dir, item.strip())
    ]
    if unexpected:
        preview = ", ".join(unexpected[:8])
        if len(unexpected) > 8:
            preview += ", ..."
        errors.append(
            f"{path} reviewed_head is stale: commits after it changed non-review files: {preview}"
        )


def is_review_admin_path(root: Path, evidence_dir: Path, changed_path: str) -> bool:
    normalized = changed_path.strip("/")
    if normalized == ".beads/issues.jsonl":
        return True

    allowed_dirs = {"docs/review/f-s7/reviews", "docs/review/f-s7/raw"}
    evidence_rel = repo_relative_dir(root, evidence_dir)
    if evidence_rel is not None:
        allowed_dirs.add(evidence_rel)
    return any(normalized.startswith(f"{directory}/") for directory in allowed_dirs)


def repo_relative_dir(root: Path, path: Path) -> str | None:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix().strip("/")
    except ValueError:
        return None


def run_git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def expect_structured_equal(
    errors: list[str], path: Path, payload: dict[str, Any], field: str, expected: Any
) -> None:
    observed = payload.get(field)
    if observed != expected:
        errors.append(f"{path} {field} must be {expected!r}, observed {observed!r}")


def non_empty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


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
