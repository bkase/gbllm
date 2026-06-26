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


PACKET_BEAD = "bd-2v9r"
DEFAULT_REVIEW_CWD = "/Users/bkase/Documents/gbllm"
DEFAULT_REVIEW_DIR = "docs/review/f-s7/reviews"
DEFAULT_BEAD_REVIEW_DIR = "docs/review/f-s7/bead-reviews"
DEFAULT_RAW_DIR = "docs/review/f-s7/raw"
DEFAULT_GEMINI_AGENT = "gemini --skip-trust -m gemini-3.1-pro-preview --acp"
DEFAULT_CLAUDE_AGENT = ""
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
REVIEWERS = ("gemini", "claude")
DEFAULT_BEAD_REVIEW_PERSONAS = ("P1", "P2", "P4", "P5", "P6", "P8")
GEMINI_AUTH_HINT_VARS = (
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "VERTEXAI_PROJECT",
    "VERTEX_AI_PROJECT",
    "CLOUDSDK_CORE_PROJECT",
    "ACPX_AUTH_GEMINI_API_KEY",
)
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
    parser.add_argument("--bead", default=PACKET_BEAD, help="bead id under review")
    parser.add_argument(
        "--bead-review",
        action="store_true",
        help=(
            "review a completed F-S7 bead and write s7_bead_acpx_review.v1 evidence "
            "instead of final bd-2v9r packet evidence"
        ),
    )
    parser.add_argument(
        "--personas",
        help=(
            "comma-separated persona ids used with --bead-review; defaults to "
            "P1,P2,P4,P5,P6,P8"
        ),
    )
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
    parser.add_argument("--bead-review-dir", default=DEFAULT_BEAD_REVIEW_DIR)
    parser.add_argument("--raw-dir", default=DEFAULT_RAW_DIR)
    parser.add_argument("--dry-run", action="store_true", help="print commands without running ACPX")
    parser.add_argument(
        "--preflight",
        action="store_true",
        help=(
            "check review cwd/head alignment and obvious reviewer-auth blockers "
            "without running ACPX or writing evidence"
        ),
    )
    parser.add_argument(
        "--skip-final-validate",
        action="store_true",
        help="skip validate-reviews.py after writing PASS evidence; intended for focused tests only",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    try:
        if not args.bead_review and args.bead != PACKET_BEAD:
            raise ReviewRunnerError("--bead requires --bead-review unless reviewing the final packet bead")
        personas = personas_from_args(args)
        head = git_head(root)
        reviewers = REVIEWERS if args.reviewer == "all" else (args.reviewer,)
        plans = [
            review_plan(
                acpx=args.acpx,
                reviewer=reviewer,
                bead=args.bead,
                bead_review=args.bead_review,
                personas=personas,
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
            print(f"# bead={plan.bead} reviewer={plan.reviewer} personas={','.join(plan.personas)}")
        print("S7 ACPX review runner: dry-run ok")
        return 0

    if args.preflight:
        return run_preflight(
            root=root,
            review_cwd=Path(args.review_cwd),
            expected_head=head,
            reviewers=reviewers,
            acpx=args.acpx,
            gemini_agent=args.gemini_agent,
            claude_agent=args.claude_agent,
        )

    try:
        validate_review_cwd_head(Path(args.review_cwd), head)
    except ReviewRunnerError as error:
        print("S7 ACPX review runner: NEEDS_CHANGES")
        print(f" - {error}")
        return 1

    errors: list[str] = []
    review_dir = output_dir(root, args.bead_review_dir if args.bead_review else args.review_dir)
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
        write_text(raw_dir / f"{plan.bead}-{plan.reviewer}.stdout.txt", completed.stdout)
        write_text(raw_dir / f"{plan.bead}-{plan.reviewer}.stderr.txt", completed.stderr)
        write_text(raw_dir / f"{plan.bead}-{plan.reviewer}.command.txt", shlex.join(plan.command) + "\n")
        if completed.returncode != 0:
            errors.append(f"{plan.reviewer} ACPX command failed with exit {completed.returncode}")
            continue

        try:
            payload = extract_review_json(completed.stdout)
            write_json(raw_dir / f"{plan.bead}-{plan.reviewer}.extracted.json", payload)
            evidence = evidence_from_payload(payload, plan, head)
        except ReviewRunnerError as error:
            errors.append(f"{plan.reviewer}: {error}")
            continue

        if evidence["verdict"] != "PASS":
            write_json(raw_dir / f"{plan.bead}-{plan.reviewer}.nonpass.json", evidence)
            errors.append(f"{plan.reviewer} review verdict was {evidence['verdict']}; not writing PASS evidence")
            continue

        review_dir.mkdir(parents=True, exist_ok=True)
        write_json(review_dir / f"{plan.bead}-{plan.reviewer}.json", evidence)

    if errors:
        print("S7 ACPX review runner: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1

    if not args.bead_review and not args.skip_final_validate and args.reviewer == "all":
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
    def __init__(
        self,
        *,
        bead: str,
        reviewer: str,
        personas: list[str],
        bead_review: bool,
        command: list[str],
        recorded_command: str,
    ) -> None:
        self.bead = bead
        self.reviewer = reviewer
        self.personas = personas
        self.bead_review = bead_review
        self.command = command
        self.recorded_command = recorded_command


def run_preflight(
    *,
    root: Path,
    review_cwd: Path,
    expected_head: str,
    reviewers: tuple[str, ...],
    acpx: str,
    gemini_agent: str,
    claude_agent: str,
) -> int:
    errors: list[str] = []
    notes: list[str] = []
    try:
        validate_review_cwd_head(review_cwd, expected_head)
        notes.append(f"review cwd HEAD matches packet root: {review_cwd}")
    except ReviewRunnerError as error:
        errors.append(str(error))

    if "gemini" in reviewers:
        if args_uses_custom_gemini_agent(gemini_agent):
            notes.append("Gemini review uses custom --gemini-agent; auth preflight delegated to that agent")
        errors.extend(gemini_preflight_errors(root, acpx, gemini_agent))
    if "claude" in reviewers:
        if claude_agent.strip():
            notes.append("Claude review uses custom --claude-agent; auth preflight delegated to that agent")
        else:
            notes.append("Claude review uses ACPX built-in claude route")

    if errors:
        print("S7 ACPX review preflight: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        for note in notes:
            print(f" - {note}")
        return 1

    print("S7 ACPX review preflight: ok")
    for note in notes:
        print(f" - {note}")
    return 0


def personas_from_args(args: argparse.Namespace) -> dict[str, list[str]]:
    if not args.bead_review:
        return {reviewer: REQUIRED_PERSONAS[reviewer] for reviewer in REVIEWERS}
    if args.personas is None:
        personas = list(DEFAULT_BEAD_REVIEW_PERSONAS)
    else:
        personas = [item.strip() for item in args.personas.split(",") if item.strip()]
    if not personas:
        raise ReviewRunnerError("--personas must name at least one persona")
    return {reviewer: personas for reviewer in REVIEWERS}


def gemini_preflight_errors(root: Path, acpx: str, gemini_agent: str) -> list[str]:
    if args_uses_custom_gemini_agent(gemini_agent):
        return []

    errors: list[str] = []
    auth_env = sorted(name for name in GEMINI_AUTH_HINT_VARS if os.environ.get(name))
    acpx_methods = acpx_auth_methods(root, acpx)
    selected_type = gemini_selected_auth_type()
    if auth_env or acpx_methods:
        return []

    detail = "no Gemini/Google/Vertex auth env vars or ACPX auth methods are configured"
    if selected_type:
        detail += f"; ~/.gemini/settings.json selectedType is {selected_type!r}"
    errors.append(
        "default Gemini ACP agent is likely to fail headlessly: "
        f"{detail}. Set a non-interactive Gemini auth route or provide S7_GEMINI_ACP_AGENT/--gemini-agent."
    )
    return errors


def args_uses_custom_gemini_agent(gemini_agent: str) -> bool:
    return gemini_agent.strip() != DEFAULT_GEMINI_AGENT


def acpx_auth_methods(root: Path, acpx: str) -> list[str]:
    completed = subprocess.run(
        [acpx, "config", "show"],
        cwd=str(root),
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if completed.returncode != 0:
        return []
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return []
    methods = payload.get("authMethods")
    if not isinstance(methods, list):
        return []
    return [str(method) for method in methods if str(method).strip()]


def gemini_selected_auth_type() -> str | None:
    settings = Path.home() / ".gemini" / "settings.json"
    if not settings.is_file():
        return None
    try:
        payload = json.loads(settings.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
    security = payload.get("security")
    if not isinstance(security, dict):
        return None
    auth = security.get("auth")
    if not isinstance(auth, dict):
        return None
    selected = auth.get("selectedType")
    return selected if isinstance(selected, str) and selected.strip() else None


def review_plan(
    *,
    acpx: str,
    reviewer: str,
    bead: str,
    bead_review: bool,
    personas: dict[str, list[str]],
    review_cwd: str,
    timeout: str,
    gemini_agent: str,
    claude_agent: str,
    head: str,
) -> ReviewPlan:
    prompt = review_prompt(reviewer, head, bead, personas[reviewer], bead_review)
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
        return ReviewPlan(
            bead=bead,
            reviewer=reviewer,
            personas=personas[reviewer],
            bead_review=bead_review,
            command=command,
            recorded_command=shlex.join(recorded),
        )
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
        return ReviewPlan(
            bead=bead,
            reviewer=reviewer,
            personas=personas[reviewer],
            bead_review=bead_review,
            command=command,
            recorded_command=shlex.join(recorded),
        )
    raise ReviewRunnerError(f"unsupported reviewer {reviewer!r}")


def review_prompt(
    reviewer: str, head: str, bead: str, personas: list[str], bead_review: bool
) -> str:
    if bead_review:
        return "\n".join(
            [
                f"Review completed F-S7 bead {bead} at git HEAD {head}.",
                "Use ACPX only; do not mutate files.",
                f"Reviewer id: {reviewer}. Required personas: {', '.join(personas)}.",
                "Inspect the current repository state, the bead record via `br show "
                f"{bead}`, history/rfcs/F-S7-moe-beats-dense.md, and the code/tests/artifacts "
                "named by the bead closure.",
                "A PASS verdict is allowed only if this bead's closure claims are supported by "
                "current evidence and no blocking finding remains. Do not treat a PASS here as "
                "final bd-2v9r production-packet closure; this is bead-level review coverage only.",
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
    return "\n".join(
        [
            f"Review F-S7 closure bead {bead} at git HEAD {head}.",
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
        "schema": "s7_bead_acpx_review.v1" if plan.bead_review else "s7_acpx_review.v1",
        "bead": plan.bead,
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


def validate_review_cwd_head(review_cwd: Path, expected_head: str) -> None:
    actual_head = git_head(review_cwd)
    if actual_head != expected_head:
        raise ReviewRunnerError(
            f"ACPX review cwd HEAD mismatch: {review_cwd} is at {actual_head}, "
            f"but packet root is at {expected_head}"
        )


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
