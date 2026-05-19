#!/usr/bin/env python3
"""F-S3 closure-readiness dry-run.

This script is intentionally a readiness/audit producer. It never closes beads
and it never claims merged-PR facts while the current objective is PR prep.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
AUDIT_TARGET = "gbf_experiments::s3::closure"
READINESS_SCHEMA = "s3_closure_readiness.v1"
B24_GATES = [
    ("s3_preregistration_check", ["scripts/s3_preregistration_check.sh"]),
    ("s3_determinism_check", ["scripts/s3_determinism_check.sh"]),
    ("s3_full_determinism_check", ["scripts/s3_full_determinism_check.sh"]),
    ("s3_isolation_check", ["scripts/s3_isolation_check.sh"]),
    ("s3_api_drift_check", ["scripts/s3_api_drift_check.sh"]),
    ("s3_oracle_re_run_check", ["scripts/s3_oracle_re_run_check.sh"]),
    ("s3_no_naming_resolution_check", ["scripts/s3_no_naming_resolution_check.sh"]),
    ("s3_feature_matrix_check", ["scripts/s3_feature_matrix_check.sh"]),
]
EXTERNAL_CLOSURE_REQUIREMENTS = [
    "S3 PR opened after all implementable RFC beads are reviewed",
    "s3-pr.yml workflow success recorded on the closure PR",
    "S3 PR merged; first_result_commit points at the first committed S3 result artifact",
    "R-Predictions ancestry verified with git merge-base",
    "bd-3k8o closure comment ready with final s3_report.v1 evidence",
    "bd-3w2 F4 carry-through closure comment ready with QAT checklist",
    "moved-acceptance bead updates ready for all referenced owners",
    "P5/P6 plus selected conditional persona reviews approved",
]
POST_CLOSE_RECORDING_REQUIREMENTS = [
    "bd-3k8o closure comment posted after human review",
    "bd-3w2 F4 carry-through closure posted after QAT closure review",
    "bd-c4wg, bd-1rcc, bd-7lu, bd-1wd, bd-3rsw, bd-2ym0, bd-tmaw, bd-3bf1, bd-2sd7 updated as applicable",
]
MOVED_ACCEPTANCE_BEADS = [
    "bd-c4wg",
    "bd-1rcc",
    "bd-7lu",
    "bd-1wd",
    "bd-3rsw",
    "bd-2ym0",
    "bd-tmaw",
    "bd-3bf1",
    "bd-2sd7",
]


class Audit:
    def __init__(self, path: Path):
        self.path = path
        self.events: list[dict[str, Any]] = []

    def emit(self, event_name: str, **fields: Any) -> None:
        self.events.append(
            {
                "target": AUDIT_TARGET,
                "event_name": event_name,
                "fields": fields,
            }
        )

    def write(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(
            "".join(
                json.dumps(event, sort_keys=True, separators=(",", ":")) + "\n"
                for event in self.events
            ),
            encoding="utf-8",
        )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="scripts/s3_closure_dry_run.sh")
    parser.add_argument("--ci-mode", choices=["fixture", "live"], default="fixture")
    parser.add_argument("--skip-ci", action="store_true")
    parser.add_argument("--dispatcher-mode", choices=["fixture", "cargo"], default="cargo")
    parser.add_argument("--report-path", default="/tmp/s3-closure-readiness.json")
    parser.add_argument("--report-dir")
    parser.add_argument("--audit-path", default="experiments/S3/closure-audit.ndjson")
    parser.add_argument(
        "--current-objective",
        choices=["pr-open-prep", "post-merge-closure"],
        default="pr-open-prep",
    )
    parser.add_argument("--pr-opened", action="store_true")
    parser.add_argument("--pr-workflow-success", action="store_true")
    parser.add_argument("--pr-merged", action="store_true")
    parser.add_argument("--r-predictions-ancestry-verified", action="store_true")
    parser.add_argument("--bd-3k8o-comment-ready", action="store_true")
    parser.add_argument("--bd-3w2-comment-ready", action="store_true")
    parser.add_argument("--moved-acceptance-comments-ready", action="store_true")
    parser.add_argument("--persona-reviews-approved", action="store_true")
    args = parser.parse_args(argv)

    report_path = Path(args.report_path)
    if args.report_dir:
        report_path = Path(args.report_dir) / "s3-closure-readiness.json"
    audit_path = resolve(args.audit_path)
    audit = Audit(audit_path)

    gate_results = [] if args.skip_ci else run_b24_gates(args, audit)
    dispatcher = evaluate_dispatcher(args.dispatcher_mode, audit)
    external_requirements = external_requirement_statuses(args)
    blockers = current_blockers(args, gate_results, dispatcher, external_requirements)
    ready_to_close = (
        not blockers
        and args.current_objective == "post-merge-closure"
        and args.ci_mode == "live"
        and args.dispatcher_mode == "cargo"
        and not args.skip_ci
    )
    report = {
        "schema": READINESS_SCHEMA,
        "current_objective": args.current_objective,
        "ready_to_close": ready_to_close,
        "script_scope": {
            "runs_b24_gate_scripts": not args.skip_ci,
            "does_not_close_beads": True,
            "does_not_claim_merged_pr": True,
            "dispatcher_is_fixture_only": args.dispatcher_mode == "fixture",
        },
        "ci_mode": args.ci_mode,
        "b24_gate_results": gate_results,
        "dispatcher": dispatcher,
        "closure_audit_path": str(audit_path),
        "external_closure_requirements": external_requirements,
        "pr_or_post_merge_followups": EXTERNAL_CLOSURE_REQUIREMENTS,
        "post_close_recording_requirements": POST_CLOSE_RECORDING_REQUIREMENTS,
        "current_blockers": blockers,
        "closure_comment_templates": closure_comment_templates(dispatcher),
        "moved_acceptance_beads": MOVED_ACCEPTANCE_BEADS,
    }
    audit.emit(
        "s3::closure::readiness_summary",
        ready_to_close=ready_to_close,
        current_objective=args.current_objective,
        blocker_count=len(blockers),
        ci_mode=args.ci_mode,
        dispatcher_mode=args.dispatcher_mode,
        report_path=str(report_path),
    )
    audit.write()
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(
        json.dumps(report, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        f"S3 closure dry-run PASS ready_to_close={str(ready_to_close).lower()} "
        f"report={report_path} audit={audit_path}"
    )
    return 0


def run_b24_gates(args: argparse.Namespace, audit: Audit) -> list[dict[str, Any]]:
    results = []
    report_dir = Path(os.environ.get("TMPDIR", "/tmp")) / "s3-closure-b24-gates"
    report_dir.mkdir(parents=True, exist_ok=True)
    for script_name, command in B24_GATES:
        full_command = list(command)
        if args.ci_mode == "fixture":
            full_command.append("--dry-run")
        if script_name == "s3_preregistration_check" and args.current_objective == "post-merge-closure":
            full_command.extend(["--result-state", "post"])
        full_command.extend(["--report-dir", str(report_dir)])
        completed = run_cmd(full_command)
        events = parse_ndjson(completed.stderr)
        summary = next(
            (event for event in events if event.get("event") == f"{script_name}_summary"),
            None,
        )
        passed = completed.returncode == 0 and summary is not None and summary.get("passed") is True
        result = {
            "script": script_name,
            "command": full_command,
            "passed": passed,
            "exit_code": completed.returncode,
            "report_path": None if summary is None else summary.get("summary", "").split("report=")[-1],
            "event_count": len(events),
        }
        results.append(result)
        audit.emit(
            "s3::closure::r_invariant_passed" if passed else "s3::closure::r_invariant_failed",
            invariant_name=f"B24:{script_name}",
            script=script_name,
            ci_mode=args.ci_mode,
            exit_code=completed.returncode,
        )
        if not passed:
            raise SystemExit(
                f"S3 closure dry-run failed B24 gate {script_name}: "
                f"stdout={completed.stdout[-1000:]} stderr={completed.stderr[-2000:]}"
            )
    return results


def evaluate_dispatcher(mode: str, audit: Audit) -> dict[str, Any]:
    if mode == "cargo":
        completed = run_cmd(
            [
                "cargo",
                "test",
                "-p",
                "gbf-experiments",
                "--features",
                "s3,s3-phase-d,s3-oracle-real",
                "--test",
                "outcome_dispatch_pass_clean_s3",
                "closure_candidate_dispatches_to_pass_clean_and_proceed",
                "--",
                "--exact",
            ]
        )
        passed = completed.returncode == 0
        mode_detail = {
            "mode": "cargo",
            "test_target": "outcome_dispatch_pass_clean_s3",
            "test_name": "closure_candidate_dispatches_to_pass_clean_and_proceed",
            "passed": passed,
        }
    else:
        passed = True
        mode_detail = {
            "mode": "fixture",
            "test_target": None,
            "test_name": None,
            "passed": True,
        }
    dispatcher = {
        **mode_detail,
        "s3_outcome": "Pass-clean",
        "s3_decision": "ProceedToS4",
        "s3_verifier_bundle_summary": {
            "source": "S3VerifierBundle::closure_candidate fixture",
            "seed_count": 5,
            "completion_cells": 15,
            "hypotheses": {
                "H1": "Confirmed",
                "H2": "Confirmed",
                "H3": "Confirmed",
                "H4": "Confirmed",
                "H5": "Confirmed",
                "H6": "Confirmed",
                "H7": "Confirmed",
            },
            "oracle_fallback_used": [],
        },
    }
    audit.emit(
        "s3::closure::dry_run_dispatcher_evaluated",
        s3_outcome=dispatcher["s3_outcome"],
        s3_decision=dispatcher["s3_decision"],
        s3_verifier_bundle_summary=dispatcher["s3_verifier_bundle_summary"],
        dispatcher_mode=mode,
        passed=passed,
    )
    if not passed:
        raise SystemExit("S3 dispatcher dry-run failed")
    return dispatcher


def current_blockers(
    args: argparse.Namespace,
    gate_results: list[dict[str, Any]],
    dispatcher: dict[str, Any],
    external_requirements: list[dict[str, Any]],
) -> list[str]:
    blockers = []
    if args.skip_ci:
        blockers.append("B24 CI gate scripts were skipped in this dry run")
    if args.ci_mode != "live":
        blockers.append("B24 CI gate scripts were not run in live mode")
    if args.dispatcher_mode != "cargo":
        blockers.append("B21 dispatcher was not executed via cargo")
    if args.current_objective != "post-merge-closure":
        blockers.append("current objective is PR-open prep, not post-merge closure")
    blockers.extend(
        requirement["description"]
        for requirement in external_requirements
        if not requirement["satisfied"]
    )
    if any(not result["passed"] for result in gate_results):
        blockers.append("one or more B24 CI gate scripts failed")
    if not dispatcher["passed"]:
        blockers.append("B21 dispatcher dry-run did not pass")
    return blockers


def external_requirement_statuses(args: argparse.Namespace) -> list[dict[str, Any]]:
    requirements = [
        (
            "pr_opened",
            "S3 PR opened after all implementable RFC beads are reviewed",
            args.pr_opened,
        ),
        (
            "pr_workflow_success",
            "s3-pr.yml workflow success recorded on the closure PR",
            args.pr_workflow_success,
        ),
        (
            "pr_merged",
            "S3 PR merged; first_result_commit points at the first committed S3 result artifact",
            args.pr_merged,
        ),
        (
            "r_predictions_ancestry_verified",
            "R-Predictions ancestry verified with git merge-base",
            args.r_predictions_ancestry_verified,
        ),
        (
            "bd_3k8o_comment_ready",
            "bd-3k8o closure comment ready with final s3_report.v1 evidence",
            args.bd_3k8o_comment_ready,
        ),
        (
            "bd_3w2_comment_ready",
            "bd-3w2 F4 carry-through closure comment ready with QAT checklist",
            args.bd_3w2_comment_ready,
        ),
        (
            "moved_acceptance_comments_ready",
            "moved-acceptance bead updates ready for all referenced owners",
            args.moved_acceptance_comments_ready,
        ),
        (
            "persona_reviews_approved",
            "P5/P6 plus selected conditional persona reviews approved",
            args.persona_reviews_approved,
        ),
    ]
    return [
        {"id": requirement_id, "description": description, "satisfied": satisfied}
        for requirement_id, description, satisfied in requirements
    ]


def closure_comment_templates(dispatcher: dict[str, Any]) -> dict[str, str]:
    return {
        "bd-3k8o": "\n".join(
            [
                "TEMPLATE ONLY - do not post until the S3 PR is merged and all reviewer gates pass.",
                "",
                "## F-S3 closure evidence",
                "- s3_report.v1: <path and SHA after merge>",
                "- predictions_commit: <from report>",
                "- first_result_commit: <from report>",
                "- report_self_hash: <from report>",
                f"- dry-run dispatcher: {dispatcher['s3_outcome']} / {dispatcher['s3_decision']}",
                "- Q1..Q6 per-seed pass matrix: <fill from final v0_success artifact>",
                "- H1..H7 verdicts: <fill from final report>",
                "- F1-broken-S3..F9-broken-S3 verdicts: <fill from falsification suite>",
                "- oracle_fallback_used: <empty or explicit fallback tags>",
                "- oracle_re_run_self_hash: <from B22/final report>",
                "- moved-acceptance cross-reference: "
                + ", ".join(MOVED_ACCEPTANCE_BEADS),
            ]
        ),
        "bd-3w2": "\n".join(
            [
                "TEMPLATE ONLY - use qat-bead-closure skill before posting.",
                "",
                "## F4 carry-through via F-S3 H7",
                "- H7 verdict: Confirmed in final s3_report.v1",
                "- Phase A/B/C/D completion: 5/5 seeds completed",
                "- Hardness ramp: PhaseCRampD2PlusPhaseDRampD2",
                "",
                "## QAT Closure Checklist",
                "- Artifact contract: <cite final S3 ReferenceModelBundle and ModelArtifact evidence>",
                "- differentiable Burn path: <cite F4/F-S3 Phase-D gate or moved owner>",
                "- Tests proving it: <cite final S3 gates>",
                "- Acceptance movement: <name moved owner beads or state none>",
                "",
                "## Support Matrix",
                "| Public behavior or variant | Scalar/model core | Burn training path | Export/artifact path | Guard |",
                "| --- | --- | --- | --- | --- |",
                "| F4 dense teacher carry-through via S3 H7 | supported | supported or moved | supported by final S3 artifacts | <fill from final gates> |",
                "",
                "## Claim-To-Gate",
                "| Closure claim | Guarding test or command | Feature gate | Notes or deviation |",
                "| --- | --- | --- | --- |",
                "| H7 carry-through confirms F4 closure condition | <final s3_report.v1 + B21/B25 gate> | s3,s3-phase-d,s3-oracle-real | Template only until PR merge |",
                "",
                "## No-future",
                "Only close F4 for QAT paths implemented and guarded today; unsupported future variants must be rejected, moved to a named owner bead, or left open.",
            ]
        ),
    }


def run_cmd(command: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return completed


def parse_ndjson(text: str) -> list[dict[str, Any]]:
    events = []
    for line in text.splitlines():
        if not line.strip():
            continue
        events.append(json.loads(line))
    return events


def resolve(raw: str | Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else ROOT / path


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
