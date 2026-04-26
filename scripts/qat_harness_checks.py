#!/usr/bin/env python3
"""Deterministic QAT harness checks for bead closure and ownership claims."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


HARNESS_START_ISO = "2026-04-26T10:47"
ISSUE_ID_RE = re.compile(r"\bbd-[a-z0-9]+\b")
CODE_SPAN_RE = re.compile(r"`([^`]+)`")
OWNER_KEYWORDS = (
    "moved to",
    "moved into",
    "owned by",
    "owns",
    "owner bead",
    "called out on",
    "follow-up bead",
)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    issues = load_issues(root / ".beads" / "issues.jsonl")
    by_id = {issue["id"]: issue for issue in issues}

    violations: list[str] = []
    violations.extend(lint_qat_closures(issues))
    violations.extend(lint_acceptance_owners(issues, by_id))

    if violations:
        print("QAT harness checks failed:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1

    print("QAT harness checks passed.")
    return 0


def load_issues(path: Path) -> list[dict[str, Any]]:
    issues = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                issues.append(json.loads(line))
    return issues


def lint_qat_closures(issues: list[dict[str, Any]]) -> list[str]:
    violations = []
    for issue in issues:
        if not is_post_harness_closed_qat_issue(issue):
            continue

        text = issue_text(issue)
        required_fragments = [
            "QAT Closure Checklist",
            "Artifact contract",
            "differentiable Burn path",
            "Tests proving it",
            "Support Matrix",
            "Claim-To-Gate",
            "| Closure claim | Guarding test or command | Feature gate | Notes or deviation |",
            "No-future",
        ]

        for fragment in required_fragments:
            if fragment not in text:
                violations.append(
                    f"{issue['id']} closed QAT bead is missing closure fragment: {fragment}"
                )

    return violations


def is_post_harness_closed_qat_issue(issue: dict[str, Any]) -> bool:
    labels = set(issue.get("labels", []))
    closed_at = issue.get("closed_at") or ""
    return (
        issue.get("status") == "closed"
        and "qat" in labels
        and closed_at >= HARNESS_START_ISO
    )


def lint_acceptance_owners(
    issues: list[dict[str, Any]], by_id: dict[str, dict[str, Any]]
) -> list[str]:
    violations = []
    for issue in issues:
        for sentence in ownership_sentences(issue_text(issue)):
            target_ids = sorted(set(ISSUE_ID_RE.findall(sentence)))
            if not target_ids:
                continue

            terms = ownership_terms(sentence)
            for target_id in target_ids:
                target = by_id.get(target_id)
                if target is None:
                    violations.append(
                        f"{issue['id']} references missing moved-acceptance owner {target_id}: {sentence}"
                    )
                    continue

                if target.get("deleted_at"):
                    violations.append(
                        f"{issue['id']} references deleted moved-acceptance owner {target_id}: {sentence}"
                    )
                    continue

                if terms and not any(term in issue_text(target) for term in terms):
                    violations.append(
                        f"{issue['id']} says {target_id} owns/moves {terms}, but that bead does not mention any of those terms"
                    )

    return violations


def ownership_sentences(text: str) -> list[str]:
    sentences = []
    for chunk in re.split(r"(?<=[.!?])\s+|[;\n]+", text):
        lowered = chunk.lower()
        if any(keyword in lowered for keyword in OWNER_KEYWORDS):
            sentences.append(chunk.strip())
    return sentences


def ownership_terms(sentence: str) -> list[str]:
    terms = []
    for term in CODE_SPAN_RE.findall(sentence):
        if term.startswith("bd-"):
            continue
        if term.startswith("cargo "):
            continue
        if "/" in term:
            continue
        if " " in term:
            continue
        terms.append(term)

    return sorted(set(terms))


def issue_text(issue: dict[str, Any]) -> str:
    parts = [
        issue.get("title", ""),
        issue.get("description", ""),
        issue.get("acceptance_criteria", ""),
        issue.get("close_reason", ""),
    ]
    parts.extend(comment.get("text", "") for comment in issue.get("comments", []))
    return "\n".join(parts)


if __name__ == "__main__":
    raise SystemExit(main())
