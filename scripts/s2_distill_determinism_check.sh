#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s2_distill_determinism_check.sh [--dry-run] [--report-path PATH] [--report-dir DIR]

Runs the F-S2 O12 distillation determinism gate. Dry-run validates the pinned
fixture shape and emits the same structured report schema. By default the
structured report is written to /tmp/s2-distill-determinism.json.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dry_run=0
report_path="/tmp/s2-distill-determinism.json"

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
            report_path="${1%/}/s2-distill-determinism.json"
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
import struct
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1])
dry_run = sys.argv[2] == "1"
report_path = Path(sys.argv[3])
script = "s2_distill_determinism_check"
EXPECTED_DISTILL_SCHEMA = "s2_distill_once_cli.v1"
EXPECTED_DISTILL_EVIDENCE_SOURCE = "gbf s2 distill-once"
stages = []
injected_failure = os.environ.get("S2_SCRIPT_INJECT_FAILURE")

class DistillEvidenceError(RuntimeError):
    def __init__(self, message, payload=None, distill=None):
        super().__init__(message)
        self.payload = payload
        self.distill = distill

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
    emit({"event": f"{script}_failure", "stage": stage, "reason": reason, "remediation": remediation})
    finish(False, 1, f"S2 distill-determinism FAIL stage={stage} reason={reason} report={report_path}")

def cli_command():
    fixture = "bad-fixture" if injected_failure == "distill_cli_failure" else "pinned"
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
        "distill-once",
        "--fixture",
        fixture,
        "--json",
    ]

def run_distill_once():
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
            "gbf s2 distill-once failed "
            + json.dumps(
                {
                    "returncode": completed.returncode,
                    "stderr": completed.stderr[-4000:],
                },
                sort_keys=True,
            )
        )
    if injected_failure == "distill_invalid_json":
        completed.stdout = "{not-json"
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"gbf s2 distill-once emitted invalid JSON: {error}") from error
    if injected_failure == "distill_bad_schema":
        payload["schema"] = "s2_distill_once_cli.v0"
    if payload.get("schema") != EXPECTED_DISTILL_SCHEMA:
        raise DistillEvidenceError(
            f"unexpected distill schema: {payload.get('schema')!r}",
            payload=payload,
            distill=payload.get("distill") if isinstance(payload.get("distill"), dict) else None,
        )
    if payload.get("evidence_source") != EXPECTED_DISTILL_EVIDENCE_SOURCE:
        raise DistillEvidenceError(
            "distill payload did not identify gbf s2 distill-once evidence",
            payload=payload,
            distill=payload.get("distill") if isinstance(payload.get("distill"), dict) else None,
        )
    distill = payload.get("distill")
    if not isinstance(distill, dict):
        raise DistillEvidenceError("distill payload missing distill object", payload=payload)
    for key in ("distill_loss_raw_bits_hex", "distill_loss_raw_sha", "distill_loss_weighted"):
        if key not in distill:
            raise DistillEvidenceError(
                f"distill payload missing {key}",
                payload=payload,
                distill=distill,
            )
    return payload

def run_distill_stage(stage_name, index):
    try:
        payload = run_distill_once()
    except Exception as error:
        payload = getattr(error, "payload", None)
        distill = getattr(error, "distill", None)
        detail = {
            "dry_run": dry_run,
            "evidence_source": "gbf s2 distill-once",
            "command": cli_command(),
            "reason": str(error),
            "failure_injection": injected_failure,
            **distill_payload_detail(payload, distill),
        }
        stage_done(stage_name, index, False, detail)
        fail(index, "distill-once CLI evidence unavailable", "inspect gbf s2 distill-once stderr/schema/JSON output")
    return payload

def distill_payload_detail(payload, distill):
    if payload is None or distill is None:
        return {
            "cli_payload_schema": None,
            "cli_payload_evidence_source": None,
            "cli_payload_fixture": None,
            "cli_payload_class_count": None,
            "cli_payload_row_count": None,
            "distill_loss_raw_sha": None,
            "distill_loss_raw_bits_hex": None,
            "distill_loss_weighted": None,
            "distill_loss_weighted_bits_hex": None,
        }
    return {
        "cli_payload_schema": payload.get("schema"),
        "cli_payload_evidence_source": payload.get("evidence_source"),
        "cli_payload_fixture": payload.get("fixture"),
        "cli_payload_class_count": distill.get("class_count"),
        "cli_payload_row_count": distill.get("row_count"),
        "distill_loss_raw_sha": distill.get("distill_loss_raw_sha"),
        "distill_loss_raw_bits_hex": distill.get("distill_loss_raw_bits_hex"),
        "distill_loss_weighted": distill.get("distill_loss_weighted"),
        "distill_loss_weighted_bits_hex": f64_bits_hex(distill.get("distill_loss_weighted")),
    }

def f64_bits_hex(value):
    if isinstance(value, (int, float)):
        return struct.pack(">d", float(value)).hex()
    return None

stage_start(1, "Validate pinned byte-deterministic DistillInputs")
distill_rs = repo / "gbf-experiments" / "src" / "s2" / "distill.rs"
if not distill_rs.exists():
    stage_done("pin_distill_inputs", 1, False, {"s2_distill_exists": False})
    fail(1, "missing S2 distill helper", "restore gbf-experiments/src/s2/distill.rs")
stage_done("pin_distill_inputs", 1, True, {
    "dry_run": dry_run,
    "evidence_source": "gbf s2 distill-once",
    "command": cli_command(),
})

stage_start(2, "Invoke deterministic distill step run 1")
first_payload = None if dry_run else run_distill_stage("distill_once_1", 2)
first = None if dry_run else first_payload["distill"]
stage_done("distill_once_1", 2, True, {
    "dry_run": dry_run,
    "evidence_source": "gbf s2 distill-once",
    "command": cli_command(),
    **distill_payload_detail(first_payload, first),
})

stage_start(3, "Invoke deterministic distill step run 2")
second_payload = None if dry_run else run_distill_stage("distill_once_2", 3)
second = None if dry_run else second_payload["distill"]
if injected_failure == "distill_mismatch" and second is not None:
    second["distill_loss_raw_sha"] = "sha256:forced-distill-mismatch"
    second["injected_failure"] = "parsed_payload_distill_sha_mutation"
stage_done("distill_once_2", 3, True, {
    "dry_run": dry_run,
    "evidence_source": "gbf s2 distill-once",
    "command": cli_command(),
    "failure_injection": None if second is None else second.get("injected_failure"),
    **distill_payload_detail(second_payload, second),
})

stage_start(4, "Assert distill_loss_raw bytes match")
mismatches = []
if not dry_run:
    for key in ("distill_loss_raw_bits_hex", "distill_loss_raw_sha", "distill_loss_weighted"):
        if first[key] != second[key]:
            mismatches.append(key)
passed = not mismatches
stage_done("bytewise_compare", 4, passed, {
    "dry_run": dry_run,
    "evidence_source": "gbf s2 distill-once",
    "mismatches": mismatches,
    "comparison_keys": ["distill_loss_raw_bits_hex", "distill_loss_raw_sha", "distill_loss_weighted"],
    "failure_injection": None if dry_run else second.get("injected_failure"),
    "run1_sha": None if dry_run else first["distill_loss_raw_sha"],
    "run2_sha": None if dry_run else second["distill_loss_raw_sha"],
})
if not passed:
    fail(4, "distill loss bytes differ", "inspect s2_distill_step inputs, temperature, and floating-point reduction order")

finish(True, 0, f"S2 distill-determinism PASS dry_run={str(dry_run).lower()} report={report_path}")
PY
