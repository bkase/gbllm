#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

fixture="${1:-dense_default}"
build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-stage4}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
fixture_dir="gbf-codegen/tests/fixtures/f_b6_f_b7/accept/stage4/$fixture"
ndjson="$out_dir/stage4-run.ndjson"

case "$fixture" in
  dense_default|moe_trace|sequence_state|bringup_minimum) ;;
  *)
    echo "error: unknown Stage 4 accept fixture '$fixture'" >&2
    exit 2
    ;;
esac

if [[ ! -d "$fixture_dir" ]]; then
  echo "error: missing Stage 4 fixture directory $fixture_dir" >&2
  exit 1
fi

mkdir -p "$out_dir/reports/stage4/$fixture"
cat >"$out_dir/stage4-run.SUBSTRATE_ONLY.txt" <<'EOF'
stage4-run.sh is a telemetry/report substrate smoke check.
It does not invoke gbf-codegen::s4::observation_plan::run_stage4, and it is not
driver-level accept/reject evidence. Real driver coverage lives in Rust tests.
EOF

python3 - "$ndjson" "$build_id" "$fixture" <<'PY'
import json
import sys
import time

path, build_id, fixture = sys.argv[1:4]
events = [
    "stage4.observation_plan.identity_bind",
    "stage4.observation_plan.schema_ingest",
    "stage4.observation_plan.build_feasibility_filter",
    "stage4.observation_plan.semantic_selection",
    "stage4.observation_plan.semantic_anchor_binding",
    "stage4.observation_plan.observation_encoding_binding",
    "stage4.observation_plan.probe_registry_instantiation",
    "stage4.observation_plan.probe_budget_governance",
    "stage4.observation_plan.probe_ordering",
    "stage4.observation_plan.metric_registry_filter",
    "stage4.observation_plan.metric_selection",
    "stage4.observation_plan.metric_ordering",
    "stage4.observation_plan.anchor_table_bind",
    "stage4.observation_plan.provenance_bind",
    "stage4.observation_plan.schema_re_emit",
    "stage4.observation_plan.operational_probe_schema_emit",
    "stage4.observation_plan.invariant_budget_check",
    "stage4.observation_plan.self_consistency",
    "stage4.observation_plan.canonical_sort",
    "stage4.driver.cache_lookup",
    "stage4.driver.cache_miss",
    "stage4.driver.report_emit",
    "stage4.driver.run",
]
with open(path, "a", encoding="utf-8") as f:
    for seq, event in enumerate(events, 1):
        f.write(json.dumps({
            "ts": f"unix:{time.time():.9f}",
            "event": event,
            "level": "INFO",
            "target": "gbf_codegen::s4",
            "fields": {
                "site_id": "dense.matmul.0",
                "checkpoint_id": "layer.0.post_embedding",
                "compact_checkpoint_id": 1,
                "stratum": "denotation",
                "probe_instance_id": "0007",
                "runtime_probe_id": 7,
                "importance_class": "Required",
                "build_id": build_id,
                "k4_hash": "sha256:" + "44" * 32,
                "k5_hash": "not-applicable:stage4",
                "outcome": "passed",
                "diag_code": "none",
                "elapsed_ns": seq,
                "event_seq": seq,
                "fixture": fixture,
                "substrate_note": "substrate-only; run_stage4 not invoked; Rust tests own driver evidence",
            },
            "span": None,
        }, sort_keys=True) + "\n")
PY

python3 - "$out_dir/reports/stage4/$fixture" "$build_id" "$fixture" <<'PY'
import json
import pathlib
import sys

report_dir = pathlib.Path(sys.argv[1])
build_id = sys.argv[2]
fixture = sys.argv[3]
payloads = {
    "observation_plan.json": {
        "schema": "observation_plan.v1",
        "fixture": fixture,
        "build_id": build_id,
        "status": "substrate-only",
        "driver_status": "run_stage4 not invoked by this script",
    },
    "semantic_checkpoint_schema.json": {
        "schema": "build_active_semantic_checkpoint_schema.v1",
        "checkpoint_count": 1,
        "driver_status": "run_stage4 not invoked by this script",
    },
    "operational_probe_schema.json": {
        "schema": "operational_probe_schema.v1",
        "probe_count": 1,
        "metric_count": 1,
        "driver_status": "run_stage4 not invoked by this script",
    },
}
for name, payload in payloads.items():
    (report_dir / name).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

echo "stage4 substrate-only smoke complete: fixture=$fixture out_dir=$out_dir"
echo "note: this script does not invoke run_stage4; use Rust driver tests for e2e evidence"
