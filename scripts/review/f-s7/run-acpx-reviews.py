#!/usr/bin/env python3
"""Run required F-S7 Gemini/Claude reviews through ACPX and materialize PASS evidence."""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any


BEAD = "bd-2v9r"
DEFAULT_REVIEW_CWD = "/Users/bkase/Documents/gbllm"
DEFAULT_REVIEW_DIR = "docs/review/f-s7/reviews"
DEFAULT_RAW_DIR = "docs/review/f-s7/raw"
DEFAULT_GEMINI_AGENT = "gemini --skip-trust -m gemini-3.1-pro-preview --acp"
DEFAULT_CLAUDE_AGENT = ""
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
REVIEWERS = ("gemini", "claude")
REQUIRED_PERSONAS = {
    "gemini": ["P3", "P4", "P5", "P6", "P7", "P8"],
    "claude": ["P3", "P5", "P6", "P8"],
}


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Run the required F-S7 bd-2v9r ACPX reviews. The script saves raw "
            "ACPX output, extracts the reviewer JSON, and writes closure review "
            "evidence only when the reviewer returns PASS."
        )
    )
    parser.add_argument("--root", default=".", help="repository root whose HEAD is reviewed")
    parser.add_argument("--review-cwd", default=DEFAULT_REVIEW_CWD, help="ACPX --cwd value")
    parser.add_argument("--reviewer", choices=[*REVIEWERS, "all"], default="all")
    parser.add_argument("--acpx", default="acpx", help="acpx executable")
    parser.add_argument("--timeout", default="1800", help="ACPX timeout in seconds")
    parser.add_argument(
        "--gemini-agent",
        default=os.environ.get("S7_GEMINI_ACP_AGENT", DEFAULT_GEMINI_AGENT),
        help=(
            "raw ACPX --agent command for Gemini review; defaults to "
            "S7_GEMINI_ACP_AGENT or the project-pinned Gemini CLI command"
        ),
    )
    parser.add_argument(
        "--claude-agent",
        default=os.environ.get("S7_CLAUDE_ACP_AGENT", DEFAULT_CLAUDE_AGENT),
        help=(
            "optional raw ACPX --agent command for Claude review; defaults to "
            "S7_CLAUDE_ACP_AGENT or the ACPX built-in `claude exec` route"
        ),
    )
    parser.add_argument("--review-dir", default=DEFAULT_REVIEW_DIR)
    parser.add_argument("--raw-dir", default=DEFAULT_RAW_DIR)
    parser.add_argument("--dry-run", action="store_true", help="print commands without running ACPX")
    parser.add_argument(
        "--skip-final-validate",
        action="store_true",
        help="skip validate-reviews.py after writing PASS evidence; intended for focused tests only",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    try:
        head = git_head(root)
        reviewers = REVIEWERS if args.reviewer == "all" else (args.reviewer,)
        plans = [
            review_plan(
                acpx=args.acpx,
                reviewer=reviewer,
                review_cwd=args.review_cwd,
                timeout=args.timeout,
                gemini_agent=args.gemini_agent,
                claude_agent=args.claude_agent,
                head=head,
            )
            for reviewer in reviewers
        ]
    except ReviewRunnerError as error:
        print("S7 ACPX review runner: NEEDS_CHANGES")
        print(f" - {error}")
        return 1

    if args.dry_run:
        for plan in plans:
            print("+ " + shlex.join(plan.command))
            print(f"# reviewer={plan.reviewer} personas={','.join(REQUIRED_PERSONAS[plan.reviewer])}")
        print("S7 ACPX review runner: dry-run ok")
        return 0

    errors: list[str] = []
    review_dir = output_dir(root, args.review_dir)
    raw_dir = output_dir(root, args.raw_dir)
    raw_dir.mkdir(parents=True, exist_ok=True)

    for plan in plans:
        completed = subprocess.run(
            plan.command,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=os.environ.copy(),
        )
        write_text(raw_dir / f"{BEAD}-{plan.reviewer}.stdout.txt", completed.stdout)
        write_text(raw_dir / f"{BEAD}-{plan.reviewer}.stderr.txt", completed.stderr)
        write_text(raw_dir / f"{BEAD}-{plan.reviewer}.command.txt", shlex.join(plan.command) + "\n")
        if completed.returncode != 0:
            errors.append(f"{plan.reviewer} ACPX command failed with exit {completed.returncode}")
            continue

        try:
            payload = extract_review_json(completed.stdout)
            write_json(raw_dir / f"{BEAD}-{plan.reviewer}.extracted.json", payload)
            evidence = evidence_from_payload(payload, plan, head)
        except ReviewRunnerError as error:
            errors.append(f"{plan.reviewer}: {error}")
            continue

        if evidence["verdict"] != "PASS":
            write_json(raw_dir / f"{BEAD}-{plan.reviewer}.nonpass.json", evidence)
            errors.append(f"{plan.reviewer} review verdict was {evidence['verdict']}; not writing PASS evidence")
            continue

        review_dir.mkdir(parents=True, exist_ok=True)
        write_json(review_dir / f"{BEAD}-{plan.reviewer}.json", evidence)

    if errors:
        print("S7 ACPX review runner: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1

    if not args.skip_final_validate and args.reviewer == "all":
        validator = Path(__file__).with_name("validate-reviews.py")
        review_dir_arg = review_dir.relative_to(root) if review_dir.is_relative_to(root) else review_dir
        command = [
            str(validator),
            "--root",
            str(root),
            "--review-dir",
            str(review_dir_arg),
            "--expected-head",
            head,
        ]
        completed = subprocess.run(command, check=False)
        if completed.returncode != 0:
            print("S7 ACPX review runner: NEEDS_CHANGES")
            print(f" - validate-reviews.py failed with exit {completed.returncode}")
            return completed.returncode

    print("S7 ACPX review runner: ok")
    return 0


class ReviewPlan:
    def __init__(self, reviewer: str, command: list[str], recorded_command: str) -> None:
        self.reviewer = reviewer
        self.command = command
        self.recorded_command = recorded_command


def review_plan(
    *,
    acpx: str,
    reviewer: str,
    review_cwd: str,
    timeout: str,
    gemini_agent: str,
    claude_agent: str,
    head: str,
) -> ReviewPlan:
    prompt = review_prompt(reviewer, head)
    if reviewer == "gemini":
        command = [
            acpx,
            "--agent",
            gemini_agent,
            "--cwd",
            review_cwd,
            "--approve-all",
            "--format",
            "text",
            "--suppress-reads",
            "--timeout",
            timeout,
            "exec",
            prompt,
        ]
        recorded = command.copy()
        recorded[0] = "acpx"
        return ReviewPlan(reviewer, command, shlex.join(recorded))
    if reviewer == "claude":
        if claude_agent.strip():
            command = [
                acpx,
                "--agent",
                claude_agent,
                "--cwd",
                review_cwd,
                "--approve-all",
                "--format",
                "text",
                "--suppress-reads",
                "--timeout",
                timeout,
                "exec",
                prompt,
            ]
        else:
            command = [
                acpx,
                "--cwd",
                review_cwd,
                "--approve-all",
                "--format",
                "text",
                "--suppress-reads",
                "--timeout",
                timeout,
                "claude",
                "exec",
                prompt,
            ]
        recorded = command.copy()
        recorded[0] = "acpx"
        return ReviewPlan(reviewer, command, shlex.join(recorded))
    raise ReviewRunnerError(f"unsupported reviewer {reviewer!r}")


def review_prompt(reviewer: str, head: str) -> str:
    personas = REQUIRED_PERSONAS[reviewer]
    return "\n".join(
        [
            f"Review F-S7 closure bead {BEAD} at git HEAD {head}.",
            "Use ACPX only; do not mutate files.",
            f"Reviewer id: {reviewer}. Required personas: {', '.join(personas)}.",
            "Inspect the current repository state, including history/rfcs/F-S7-moe-beats-dense.md, "
            "scripts/review/f-s7/verify-packet.sh, docs/experiments/S7-report.md if present, "
            "experiments/S7/** if present, and docs/review/f-s7/**.",
            "Run or inspect scripts/review/f-s7/verify-packet.sh as needed. A PASS verdict is allowed "
            "only if the production packet and closure evidence are present and no blocking finding remains. "
            "If production artifacts, final report, RFC finalization, or required review evidence are missing, "
            "return NEEDS_CHANGES.",
            "Return exactly one JSON object. No Markdown wrapper. Shape:",
            "{",
            '  "verdict": "PASS or NEEDS_CHANGES",',
            f'  "personas": {json.dumps(personas)},',
            '  "summary": "one concise paragraph",',
            '  "findings": [',
            '    {"severity": "p1|p2|p3|info", "status": "open|resolved|non_blocking", "body": "finding text"}',
            "  ]",
            "}",
        ]
    )


def extract_review_json(text: str) -> dict[str, Any]:
    decoder = json.JSONDecoder()
    for index, char in enumerate(text):
        if char != "{":
            continue
        try:
            payload, _ = decoder.raw_decode(text[index:])
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict) and "verdict" in payload:
            return payload
    raise ReviewRunnerError("ACPX output did not contain a review JSON object")


def evidence_from_payload(payload: dict[str, Any], plan: ReviewPlan, head: str) -> dict[str, Any]:
    verdict = payload.get("verdict")
    if verdict not in {"PASS", "NEEDS_CHANGES"}:
        raise ReviewRunnerError("review verdict must be PASS or NEEDS_CHANGES")
    personas = payload.get("personas")
    if not isinstance(personas, list) or not all(isinstance(item, str) for item in personas):
        raise ReviewRunnerError("review personas must be a list of persona ids")
    summary = payload.get("summary")
    if not isinstance(summary, str) or not summary.strip():
        raise ReviewRunnerError("review summary must be a non-empty string")
    findings = payload.get("findings")
    if not isinstance(findings, list):
        raise ReviewRunnerError("review findings must be a list")
    for index, finding in enumerate(findings):
        if not isinstance(finding, dict):
            raise ReviewRunnerError(f"review findings[{index}] must be an object")

    return {
        "schema": "s7_acpx_review.v1",
        "bead": BEAD,
        "reviewer": plan.reviewer,
        "transport": "acpx",
        "verdict": verdict,
        "personas": personas,
        "command": plan.recorded_command,
        "reviewed_head": head,
        "summary": summary.strip(),
        "findings": findings,
    }


def output_dir(root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else root / path


def git_head(root: Path) -> str:
    completed = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise ReviewRunnerError(f"could not resolve git HEAD under {root}: {completed.stderr.strip()}")
    head = completed.stdout.strip()
    if not COMMIT_RE.match(head):
        raise ReviewRunnerError(f"git HEAD must be a 40-hex commit id, observed {head!r}")
    return head


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")


class ReviewRunnerError(RuntimeError):
    pass


if __name__ == "__main__":
    sys.exit(main())
