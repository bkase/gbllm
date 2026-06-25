#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-assemble-packet-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

manifest="$tmp/bundle/manifest.json"
mkdir -p "$(dirname "$manifest")"

python3 - "$manifest" <<'PY'
from pathlib import Path
import json
import sys

manifest = Path(sys.argv[1])
topologies = ["MoeTiny", "MoeTinyDenseMatched"]
seeds = range(5)

runs = {}
for topology in topologies:
    runs[topology] = {}
    for seed in seeds:
        base = f"runs/{topology}/seed-{seed}"
        runs[topology][str(seed)] = {
            "run_log": f"{base}/run-log.json",
            "score": f"{base}/score.json",
            "grad_log": f"{base}/grad-log.jsonl",
            "router_step_telemetry": f"{base}/router-step-telemetry.jsonl",
        }

payload = {
    "schema": "s7_production_bundle_manifest.v1",
    "runs": runs,
    "switch_stats": {str(seed): f"switch-stats/seed-{seed}.json" for seed in seeds},
    "support_artifacts": {
        "router_collapse_sweep": "router-collapse/seed-0/sweep.json",
        "burn_grad_smoke": "burn-grad/expert_block_qat.json",
        "oracle_routed": "oracle-routed/seed-0/oracle.json",
        "emulator_one_token_moe": "emulator-one-token/seed-0/MoeTiny/result.json",
        "emulator_one_token_dense": "emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
    },
    "comparison": {
        "moe_topology_hash": "sha256:" + "1" * 64,
        "dense_matched_topology_hash": "sha256:" + "2" * 64,
    },
    "frontier": {
        "moe_conformance": "frontier/moe-conformance.json",
        "dense_conformance": "frontier/dense-conformance.json",
        "moe_deployed_bytes_per_block": [20944, 20944, 20944, 20944],
        "dense_deployed_bytes_per_block": [20948, 20948, 20948, 20948],
        "moe_schedule_cost": "frontier/moe-schedule-cost.json",
        "dense_schedule_cost": "frontier/dense-schedule-cost.json",
    },
    "report": {
        "s7_outcome": "PassClean",
        "predictions_section_hash": "sha256:" + "3" * 64,
        "predictions_commit": "4" * 40,
        "first_result_commit": "5" * 40,
        "rfc_revision": "6" * 40,
        "generated_at": "2026-06-25T00:00:00Z",
    },
}
manifest.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates \
  --dry-run >"$tmp/dry-run.out"

rg -c " materialize-run " "$tmp/dry-run.out" | rg '^10$' >/dev/null
rg -c " --kind switch-stats " "$tmp/dry-run.out" | rg '^5$' >/dev/null
rg -n "derive-summaries" "$tmp/dry-run.out" >/dev/null
rg -n "derive-comparison" "$tmp/dry-run.out" >/dev/null
rg -n "derive-frontier" "$tmp/dry-run.out" >/dev/null
rg -n "emit-report" "$tmp/dry-run.out" >/dev/null
rg -n "verify-packet\\.sh --skip-gates" "$tmp/dry-run.out" >/dev/null
rg -n -- "--topology MoeTinyDenseMatched" "$tmp/dry-run.out" >/dev/null
rg -n -- "$tmp/bundle/runs/MoeTiny/seed-0/run-log\\.json" "$tmp/dry-run.out" >/dev/null
rg -n "S7 packet assembly: dry-run ok" "$tmp/dry-run.out" >/dev/null

python3 - "$manifest" "$tmp/bad-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["comparison"]["moe_topology_hash"] = "not-a-hash"
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/bad-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/bad.out" 2>&1; then
  echo "expected invalid comparison hash to fail" >&2
  exit 1
fi
rg -n "comparison\\.moe_topology_hash must be a sha256 hash" "$tmp/bad.out" >/dev/null

echo "s7_assemble_packet_test: ok"
