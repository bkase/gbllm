#!/usr/bin/env python3
"""Validate F-S7 Gemini/Claude ACPX review evidence."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
ACPX_COMMAND_RE = re.compile(r"^\s*acpx(?:\s|$)", re.IGNORECASE)
DEFAULT_REVIEW_DIR = "docs/review/f-s7/reviews"
ALWAYS_ON_PERSONAS = {"P5", "P6"}
REQUIRED = {
    "gemini": {"P3", "P4", "P5", "P6", "P7", "P8"},
    "claude": {"P3", "P5", "P6", "P8"},
}


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate required F-S7 bd-2v9r Gemini/Claude ACPX review evidence."
    )
    parser.add_argument("--root", default=".", help="repository root or packet root")
    parser.add_argument("--bead", default="bd-2v9r")
    parser.add_argument("--review-dir", default=DEFAULT_REVIEW_DIR)
    parser.add_argument(
        "--expected-head",
        help="40-hex git commit that every review evidence file must name as reviewed_head",
    )
    args = parser.parse_args()

    errors = validate_reviews(Path(args.root), args.bead, args.review_dir, args.expected_head)
    if errors:
        print("S7 ACPX review evidence: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1
    print("S7 ACPX review evidence: ok")
    return 0


def validate_reviews(
    root: Path, bead: str, review_dir: str, expected_head: str | None
) -> list[str]:
    errors: list[str] = []
    if expected_head is not None and not COMMIT_RE.match(expected_head):
        errors.append(f"expected_head must be a 40-hex commit id, observed {expected_head!r}")
    base = root / review_dir
    for reviewer, required_personas in REQUIRED.items():
        path = base / f"{bead}-{reviewer}.json"
        payload = load_review(errors, path)
        if payload is None:
            continue
        validate_review_payload(
            errors, path, payload, bead, reviewer, required_personas, expected_head
        )
    return errors


def load_review(errors: list[str], path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        errors.append(f"missing review evidence: {path}")
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        errors.append(f"{path} is not valid JSON: {error}")
        return None
    if not isinstance(payload, dict):
        errors.append(f"{path} must contain a JSON object")
        return None
    return payload


def validate_review_payload(
    errors: list[str],
    path: Path,
    payload: dict[str, Any],
    bead: str,
    reviewer: str,
    required_personas: set[str],
    expected_head: str | None,
) -> None:
    expect_equal(errors, path, payload, "schema", "s7_acpx_review.v1")
    expect_equal(errors, path, payload, "bead", bead)
    expect_equal(errors, path, payload, "reviewer", reviewer)
    expect_equal(errors, path, payload, "transport", "acpx")
    expect_equal(errors, path, payload, "verdict", "PASS")
    if not ACPX_COMMAND_RE.search(str(payload.get("command", ""))):
        errors.append(f"{path} command must record an ACPX invocation prefix")
    reviewed_head = str(payload.get("reviewed_head", ""))
    if not COMMIT_RE.match(reviewed_head):
        errors.append(f"{path} reviewed_head must be a 40-hex commit id")
    elif expected_head is not None and reviewed_head != expected_head:
        errors.append(
            f"{path} reviewed_head must match expected_head {expected_head}, observed {reviewed_head}"
        )
    if not non_empty_string(payload.get("summary")):
        errors.append(f"{path} summary must be a non-empty string")
    personas = payload.get("personas")
    if not isinstance(personas, list) or not all(isinstance(item, str) for item in personas):
        errors.append(f"{path} personas must be a list of persona ids")
        observed: set[str] = set()
    else:
        observed = set(personas)
    missing = sorted((required_personas - ALWAYS_ON_PERSONAS) - observed)
    if missing:
        errors.append(f"{path} missing required personas for {reviewer}: {missing}")
    for always in sorted(ALWAYS_ON_PERSONAS):
        if always not in observed:
            errors.append(f"{path} missing always-on persona {always}")
    findings = payload.get("findings")
    if not isinstance(findings, list):
        errors.append(f"{path} findings must be a list")
    elif payload.get("verdict") == "PASS":
        validate_pass_findings(errors, path, findings)


def expect_equal(
    errors: list[str], path: Path, payload: dict[str, Any], field: str, expected: Any
) -> None:
    observed = payload.get(field)
    if observed != expected:
        errors.append(f"{path} {field} must be {expected!r}, observed {observed!r}")


def non_empty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def validate_pass_findings(errors: list[str], path: Path, findings: list[Any]) -> None:
    blocking_severities = {"blocker", "critical", "major", "high", "p0", "p1"}
    allowed_severities = {
        "info",
        "low",
        "medium",
        "minor",
        "major",
        "high",
        "critical",
        "blocker",
        "p0",
        "p1",
        "p2",
        "p3",
        "p4",
    }
    resolved_statuses = {"resolved", "accepted", "false_positive", "non_blocking"}
    allowed_statuses = resolved_statuses | {"open", "unresolved"}
    for index, finding in enumerate(findings):
        if not isinstance(finding, dict):
            errors.append(f"{path} PASS review finding findings[{index}] must be an object")
            continue
        severity = str(finding.get("severity", "")).strip().lower()
        status = str(finding.get("status", "")).strip().lower()
        if severity not in allowed_severities:
            errors.append(
                f"{path} PASS review finding findings[{index}] severity must be one of {sorted(allowed_severities)}"
            )
        if status not in allowed_statuses:
            errors.append(
                f"{path} PASS review finding findings[{index}] status must be one of {sorted(allowed_statuses)}"
            )
        if severity in blocking_severities and status not in resolved_statuses:
            errors.append(
                f"{path} PASS review has unresolved blocking finding at findings[{index}]"
            )


if __name__ == "__main__":
    sys.exit(main())
