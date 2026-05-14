#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s2_isolation_check.sh [--dry-run] [--report-path PATH] [--report-dir DIR]

Runs the F-S2 O9 seed/build isolation gate. By default the structured report is
written to /tmp/s2-isolation.json. That default /tmp file is convenient for
serial local runs but can collide under parallel jobs; use --report-path or
--report-dir to give each job an isolated output path. The live path launches a
single nested cargo evidence probe serially and then compares the parsed JSON
payload, so failure injection mutates parsed evidence rather than CLI bytes.
Dry-run validates the gate inputs and emits the same report schema without
claiming final_checkpoint_sha evidence.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dry_run=0
report_path="/tmp/s2-isolation.json"

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
            report_path="${1%/}/s2-isolation.json"
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
import tempfile
from pathlib import Path

repo = Path(sys.argv[1])
dry_run = sys.argv[2] == "1"
report_path = Path(sys.argv[3])
script = "s2_isolation_check"
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
    finish(False, 1, f"S2 isolation FAIL stage={stage} reason={reason} report={report_path}")

def collect_evidence():
    with tempfile.TemporaryDirectory(prefix="s2-isolation-") as tmp:
        result_path = Path(tmp) / "evidence.json"
        env = os.environ.copy()
        env["S2_ISOLATION_EVIDENCE_JSON"] = str(result_path)
        command = [
            "cargo", "test", "-p", "gbf-experiments", "--features", "s2-full",
            "--test", "cli_scripts_s2", "__s2_isolation_evidence_probe", "--", "--ignored", "--exact",
        ]
        output = subprocess.run(command, cwd=repo, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if output.returncode != 0:
            stage_done("collect_real_evidence", 2, False, {"status": output.returncode, "stderr_tail": output.stderr[-2000:]})
            fail(2, "tiny S2 evidence probe failed", "inspect gbf-experiments/tests/cli_scripts_s2.rs::__s2_isolation_evidence_probe")
        evidence = json.loads(result_path.read_text(encoding="utf-8"))
    if os.environ.get("S2_ISOLATION_FORCE_SHARED_STATE") == "1":
        # Test-only parsed-payload mutation: keep the live CLI/test probe path
        # intact, then make the report evidence look like a seed leak.
        evidence["seed_hashes"]["1"] = evidence["seed_hashes"]["0"]
    if os.environ.get("S2_SCRIPT_INJECT_FAILURE") == "order_dependence":
        # Test-only parsed-payload mutation: keep the live CLI/test probe path
        # intact, then make one build-order observation diverge.
        evidence["order_b"]["s2_ternary_full:0"] = evidence["seed_hashes"]["1"]
    return evidence

def validate_hashes(mapping, stage):
    bad = {
        key: value
        for key, value in mapping.items()
        if not isinstance(value, str) or not value.startswith("sha256:") or value == "sha256:" + "0" * 64
    }
    if bad:
        fail(stage, "invalid final_checkpoint_sha evidence", "consume completed S2 run products with real checkpoint hashes")

stage_start(1, "Validate isolation gate inputs")
if not (repo / "gbf-experiments" / "src" / "s2" / "run.rs").exists():
    stage_done("inputs", 1, False, {"s2_run_exists": False})
    fail(1, "missing S2 run helper", "restore gbf-experiments/src/s2/run.rs")
stage_done("inputs", 1, True, {"dry_run": dry_run})

if dry_run:
    finish(True, 0, f"S2 isolation PASS dry_run=true report={report_path}")

stage_start(2, "Collect real tiny S2 final_checkpoint_sha evidence")
evidence = collect_evidence()
seed_hashes = evidence["seed_hashes"]
expected_by_key = evidence["expected_by_key"]
order_a = evidence["order_a"]
order_b = evidence["order_b"]
validate_hashes(seed_hashes, 2)
validate_hashes(expected_by_key, 2)
validate_hashes(order_a, 2)
validate_hashes(order_b, 2)
stage_done(
    "collect_real_evidence",
    2,
    True,
    {
        "evidence_source": evidence.get("evidence_source", "tiny_s2_run"),
        "seed_hash_count": len(seed_hashes),
        "expected_by_key_count": len(expected_by_key),
        "order_a_count": len(order_a),
        "order_b_count": len(order_b),
        "stateful_seam": evidence.get("stateful_seam"),
    },
)

stage_start(3, "Require seeds 0 and 1 to produce distinct final checkpoints")
distinct_count = len(set(seed_hashes.values()))
passed = distinct_count >= 2
stage_done(
    "seed_distinctness",
    3,
    passed,
    {
        "evidence_source": evidence.get("evidence_source", "tiny_s2_run"),
        "distinct_final_checkpoint_sha_count": distinct_count,
        "seed_hashes": seed_hashes,
    },
)
if not passed:
    fail(3, "seed hashes are not distinct", "check RNG stream isolation and seed plumbing")

stage_start(4, "Assert per-build per-seed hashes are independent of build order")
mismatches = sorted(
    key
    for key, value in expected_by_key.items()
    if order_a.get(key) != value or order_b.get(key) != value
)
passed = not mismatches
stage_done(
    "order_invariance",
    4,
    passed,
    {
        "evidence_source": evidence.get("evidence_source", "tiny_s2_run"),
        "mismatches": mismatches,
        "expected_by_key": expected_by_key,
        "ternary_then_fp": order_a,
        "fp_then_ternary": order_b,
        "stateful_seam": evidence.get("stateful_seam"),
    },
)
if not passed:
    fail(4, "build order dependence", "remove shared mutable state across S2 build replays")

finish(True, 0, f"S2 isolation PASS dry_run=false distinct_final_checkpoint_sha_count={distinct_count} report={report_path}")
PY
