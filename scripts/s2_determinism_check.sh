#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s2_determinism_check.sh [--dry-run] [--report-path PATH] [--report-dir DIR]

Runs the F-S2 O2 determinism gate. Dry-run validates the gate inputs and
emits the same structured report schema without consuming train compute. By
default the structured report is written to /tmp/s2-determinism.json. The
default /tmp report path is for serial local use; use --report-path/--report-dir
for parallel jobs.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dry_run=0
report_path="/tmp/s2-determinism.json"

while (($#)); do
    case "$1" in
        --dry-run)
            dry_run=1
            ;;
        --report-path)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --report-path requires a value" >&2
                exit 2
            fi
            report_path="$1"
            ;;
        --report-dir)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --report-dir requires a value" >&2
                exit 2
            fi
            report_path="${1%/}/s2-determinism.json"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

python3 - "$repo_root" "$dry_run" "$report_path" <<'PY'
import json
import os
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1])
dry_run = sys.argv[2] == "1"
report_path = Path(sys.argv[3])
script = "s2_determinism_check"
EXPECTED_REPLAY_SCHEMA = "s2_replay_full_cli.v1"
EXPECTED_REPLAY_EVIDENCE_SOURCE = "gbf s2 replay-full"
# Keep this shell-side check centralized and intentionally loud. The script is
# a standalone CI gate, so it cannot import Rust schema constants directly.
EXPECTED_PHASE_BOUNDARY_STEPS = ("4000", "5000", "8000", "10000")
stages = []

def emit(payload):
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")), file=sys.stderr)

def stage_start(index, description):
    emit({"event": f"{script}_stage_start", "stage": index, "description": description})

def stage_done(name, index, passed, detail):
    stages.append({"name": name, "passed": passed, "detail": detail})
    emit({"event": f"{script}_stage_done", "stage": index, "passed": passed, "detail": detail})

def finish(passed, code, summary):
    report = {
        "script": script,
        "passed": passed,
        "stages": stages,
        "exit_code": code,
        "dry_run": dry_run,
        "evidence_mode": "dry_run" if dry_run else "live",
        "live_evidence": not dry_run,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    emit({"event": f"{script}_exit", "exit_code": code, "passed": passed, "summary": summary})
    print(summary)
    raise SystemExit(code)

def fail(stage, reason, remediation):
    if stages and stages[-1]["passed"]:
        stages[-1]["passed"] = False
        stages[-1]["detail"] = {**stages[-1]["detail"], "reason": reason}
    emit({"event": f"{script}_failure", "stage": stage, "reason": reason, "remediation": remediation})
    finish(False, 1, f"S2 determinism FAIL stage={stage} reason={reason} report={report_path}")

def cli_command():
    return [
        "cargo",
        "run",
        "--quiet",
        "-p",
        "gbf-cli",
        "--features",
        "s2-full",
        "--",
        "s2",
        "replay-full",
        "--seed-list",
        "0",
        "--builds",
        "s2_ternary_full",
        "--fixture",
        "tiny",
        "--json",
    ]

def run_replay():
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    completed = subprocess.run(
        cli_command(),
        cwd=repo,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "gbf s2 replay-full failed "
            + json.dumps(
                {
                    "returncode": completed.returncode,
                    "stderr": completed.stderr[-4000:],
                },
                sort_keys=True,
            )
        )
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"gbf s2 replay-full emitted invalid JSON: {error}") from error
    if payload.get("schema") != EXPECTED_REPLAY_SCHEMA:
        raise RuntimeError(f"unexpected replay schema: {payload.get('schema')!r}")
    if payload.get("evidence_source") != EXPECTED_REPLAY_EVIDENCE_SOURCE:
        raise RuntimeError("replay payload did not identify gbf s2 replay-full evidence")
    runs = payload.get("runs")
    if not isinstance(runs, list) or len(runs) != 1:
        raise RuntimeError("replay payload must contain exactly one run")
    checkpoints = runs[0].get("checkpoints", {})
    phase_boundary_steps = runs[0].get("phase_boundary_steps")
    if phase_boundary_steps != list(EXPECTED_PHASE_BOUNDARY_STEPS):
        raise RuntimeError(f"unexpected replay phase-boundary steps: {phase_boundary_steps!r}")
    missing = [step for step in EXPECTED_PHASE_BOUNDARY_STEPS if step not in checkpoints]
    if missing:
        raise RuntimeError(f"replay payload missing phase-boundary checkpoints: {missing}")
    return payload

def replay_payload_detail(payload, run):
    if payload is None or run is None:
        return {
            "cli_payload_schema": None,
            "cli_payload_evidence_source": None,
            "cli_payload_fixture": None,
            "cli_payload_seed": None,
            "cli_payload_build_kind": None,
            "cli_payload_phase_boundary_steps": [],
            "checkpoint_count": 0,
        }
    return {
        "cli_payload_schema": payload["schema"],
        "cli_payload_evidence_source": payload["evidence_source"],
        "cli_payload_fixture": payload["fixture"],
        "cli_payload_seed": run["seed"],
        "cli_payload_build_kind": run["build_kind"],
        "cli_payload_phase_boundary_steps": run["phase_boundary_steps"],
        "checkpoint_count": len(run["checkpoints"]),
    }

try:
    stage_start(1, "Validate deterministic gate inputs")
    gbf_cli = repo / "gbf-cli" / "src" / "main.rs"
    s2_cli = repo / "gbf-experiments" / "src" / "s2" / "cli.rs"
    if not gbf_cli.exists() or not s2_cli.exists():
        stage_done("inputs", 1, False, {"gbf_cli_exists": gbf_cli.exists(), "s2_cli_exists": s2_cli.exists()})
        fail(1, "missing S2 CLI surface", "restore gbf-cli and gbf-experiments S2 CLI sources")
    stage_done("inputs", 1, True, {"dry_run": dry_run, "command": cli_command()})

    stage_start(2, "Replay seed 0 s2_ternary_full run 1")
    run1_payload = None if dry_run else run_replay()
    run1 = None if dry_run else run1_payload["runs"][0]
    stage_done("replay_run_1", 2, True, {
        "dry_run": dry_run,
        "evidence_source": "gbf s2 replay-full",
        "command": cli_command(),
        **replay_payload_detail(run1_payload, run1),
    })

    stage_start(3, "Replay seed 0 s2_ternary_full run 2")
    run2_payload = None if dry_run else run_replay()
    run2 = None if dry_run else run2_payload["runs"][0]
    if os.environ.get("S2_DETERMINISM_PERTURB_LOCK_MIDRUN") == "1" and run2 is not None:
        # Failure injection mutates the parsed CLI payload after the live
        # replay succeeds. Success mode still proves bytewise live CLI
        # determinism; this branch only proves the comparator trips.
        run2["checkpoints"]["10000"] = "sha256:forced-determinism-perturbation"
        run2["injected_failure"] = "parsed_payload_checkpoint_mutation"
    if os.environ.get("S2_SCRIPT_INJECT_FAILURE") == "determinism_mismatch":
        if run2 is not None:
            # Same parsed-payload mutation pattern for the self-hash comparator.
            run2["score_self_hash"] = "sha256:forced-determinism-mismatch"
            run2["injected_failure"] = "parsed_payload_score_hash_mutation"
    stage_done("replay_run_2", 3, True, {
        "dry_run": dry_run,
        "evidence_source": "gbf s2 replay-full",
        "command": cli_command(),
        "failure_injection": None if run2 is None else run2.get("injected_failure"),
        **replay_payload_detail(run2_payload, run2),
    })

    stage_start(4, "Assert bytewise equality at S2 phase boundaries and self-hashes")
    mismatches = []
    if not dry_run:
        for step in EXPECTED_PHASE_BOUNDARY_STEPS:
            if run1["checkpoints"][step] != run2["checkpoints"][step]:
                mismatches.append(f"checkpoint_{step}")
        for key in ("final_checkpoint_sha", "phase_log_self_hash", "distill_log_self_hash", "score_self_hash"):
            if run1[key] != run2[key]:
                mismatches.append(key)
    passed = not mismatches
    stage_done("bytewise_compare", 4, passed, {
        "dry_run": dry_run,
        "evidence_source": "gbf s2 replay-full",
        "mismatches": mismatches,
        "comparison_keys": [f"checkpoint_{step}" for step in EXPECTED_PHASE_BOUNDARY_STEPS] + [
            "final_checkpoint_sha",
            "phase_log_self_hash",
            "distill_log_self_hash",
            "score_self_hash",
        ],
        "failure_injection": None if dry_run else run2.get("injected_failure"),
    })
    if not passed:
        fail(4, "determinism mismatch", "compare the mismatched S2 artifact bytes and hidden environment hashes")
    finish(True, 0, f"S2 determinism PASS dry_run={str(dry_run).lower()} report={report_path}")
except Exception as error:
    emit({"event": f"{script}_failure", "stage": len(stages) + 1, "reason": str(error), "remediation": "inspect script traceback and workspace inputs"})
    finish(False, 1, f"S2 determinism FAIL reason={error} report={report_path}")
PY
