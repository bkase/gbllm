#!/usr/bin/env python3
"""Deterministic QAT harness checks for bead closure and ownership claims."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any


HARNESS_START_ISO = "2026-04-26T10:47"
STRICT_QAT_CLAIM_START_ISO = "2026-04-26T11:18"
REVIEW_GUARDRAIL_START_ISO = "2026-04-26T12:00"
ISSUE_ID_RE = re.compile(r"\bbd-[a-z0-9]+\b")
CODE_SPAN_RE = re.compile(r"`([^`]+)`")
LOWERING_CLAIM_RE = re.compile(
    r"(exact(?:ly)?\s+match(?:es|ing)?\s+(?:the\s+)?(?:compiler|runtime|lowering)"
    r"|(?:compiler|runtime)\s+lowering\s+(?:agreement|exact))",
    re.IGNORECASE,
)
LOWERING_OWNER_RE = re.compile(r"\bbd-(?:g90|12c)\b")
LOWERING_GATE_RE = re.compile(
    r"`(?:cargo test -p gbf-(?:codegen|test|verify|oracle)[^`]*"
    r"|cargo test --workspace --all-features[^`]*)`"
)
FIRST_CLASS_TENSOR_RE = re.compile(r"\bfirst-class tensors?\b", re.IGNORECASE)
FIRST_CLASS_OWNER_RE = re.compile(r"\bbd-(?:g90|209)\b|CanonicalTensor")
BURN_MOVED_ROW_GENERIC_OWNER_RE = re.compile(
    r"^\|[^|]*\|[^|]*\|\s*moved\s*\|[^|]*\|[^|]*\bbd-1mv\b",
    re.IGNORECASE,
)
ROUTER_BALANCE_CLAIM_RE = re.compile(
    r"\b(?:balance_loss|balance loss|load-balance loss|load balance loss)\b",
    re.IGNORECASE,
)
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
    violations.extend(lint_strict_qat_claims(issues))
    violations.extend(lint_acceptance_owners(issues, by_id))
    violations.extend(lint_review_guardrails(issues, by_id))

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


def lint_strict_qat_claims(issues: list[dict[str, Any]]) -> list[str]:
    violations = []
    for issue in issues:
        if not is_strict_closed_qat_issue(issue):
            continue

        closure = closure_text(issue)
        if LOWERING_CLAIM_RE.search(closure) and not (
            LOWERING_GATE_RE.search(closure) or LOWERING_OWNER_RE.search(closure)
        ):
            violations.append(
                f"{issue['id']} claims exact compiler/runtime lowering without a codegen/oracle gate or moved owner bd-g90/bd-12c"
            )

        if FIRST_CLASS_TENSOR_RE.search(closure) and not FIRST_CLASS_OWNER_RE.search(closure):
            violations.append(
                f"{issue['id']} claims first-class tensor export without CanonicalTensor or moved owner bd-g90/bd-209"
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


def is_strict_closed_qat_issue(issue: dict[str, Any]) -> bool:
    labels = set(issue.get("labels", []))
    closed_at = issue.get("closed_at") or ""
    return (
        issue.get("status") == "closed"
        and "qat" in labels
        and closed_at >= STRICT_QAT_CLAIM_START_ISO
    )


def is_review_guardrail_closed_qat_issue(issue: dict[str, Any]) -> bool:
    labels = set(issue.get("labels", []))
    closed_at = issue.get("closed_at") or ""
    return (
        issue.get("status") == "closed"
        and "qat" in labels
        and closed_at >= REVIEW_GUARDRAIL_START_ISO
    )


def lint_review_guardrails(
    issues: list[dict[str, Any]], by_id: dict[str, dict[str, Any]]
) -> list[str]:
    violations = []
    for issue in issues:
        if not is_review_guardrail_closed_qat_issue(issue):
            continue

        text = issue_text(issue)
        violations.extend(lint_generic_burn_adapter_owner(issue, text, by_id))
        violations.extend(lint_unqualified_router_balance_claim(issue, text))

    return violations


def lint_generic_burn_adapter_owner(
    issue: dict[str, Any], text: str, by_id: dict[str, dict[str, Any]]
) -> list[str]:
    violations = []
    for line in text.splitlines():
        if not BURN_MOVED_ROW_GENERIC_OWNER_RE.search(line):
            continue

        if has_concrete_router_expert_burn_owner(text, by_id):
            continue

        violations.append(
            f"{issue['id']} moves a Burn training support-matrix row to generic closed owner bd-1mv; "
            "create/cite a concrete router/expert Burn adapter owner bead instead"
        )

    return violations


def has_concrete_router_expert_burn_owner(
    text: str, by_id: dict[str, dict[str, Any]]
) -> bool:
    owner_ids = sorted(set(ISSUE_ID_RE.findall(text)))
    for owner_id in owner_ids:
        owner = by_id.get(owner_id)
        if owner is None or owner.get("deleted_at") or owner.get("status") == "closed":
            continue

        owner_text = issue_text(owner)
        if (
            "Top1RouterQat" in owner_text
            and "ExpertBlockQat" in owner_text
            and "Burn adapter" in owner_text
        ):
            return True

    return False


def lint_unqualified_router_balance_claim(
    issue: dict[str, Any], text: str
) -> list[str]:
    if "Top1RouterQat" not in text:
        return []

    if not ROUTER_BALANCE_CLAIM_RE.search(text):
        return []

    lowered = text.lower()
    if "proxy" in lowered and "bd-1b3" in text:
        return []

    return [
        f"{issue['id']} claims router balance/load-balance behavior without naming it as a proxy "
        "or citing bd-1b3 as the standard batch/token loss owner"
    ]


def lint_acceptance_owners(
    issues: list[dict[str, Any]], by_id: dict[str, dict[str, Any]]
) -> list[str]:
    violations = []
    for issue in issues:
        if issue.get("status") != "closed":
            continue

        for sentence in ownership_sentences(issue_text(issue)):
            target_ids = sorted(set(ISSUE_ID_RE.findall(sentence)))
            if not target_ids:
                continue

            terms = ownership_terms(sentence)
            for target_id in target_ids:
                if is_explicitly_negated_reference(sentence, target_id):
                    continue

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


def is_explicitly_negated_reference(sentence: str, target_id: str) -> bool:
    escaped = re.escape(target_id)
    negated_patterns = [
        rf"\bnot\s+['\"`]?{escaped}['\"`]?",
        rf"\binstead\s+of\s+['\"`]?{escaped}['\"`]?",
    ]
    return any(re.search(pattern, sentence, re.IGNORECASE) for pattern in negated_patterns)


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


def closure_text(issue: dict[str, Any]) -> str:
    parts = [issue.get("close_reason", "")]
    parts.extend(comment.get("text", "") for comment in issue.get("comments", []))
    return "\n".join(parts)


if __name__ == "__main__":
    raise SystemExit(main())
