#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/s5_closure_rehearsal.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

bash -n "$SCRIPT"
"$SCRIPT" --self-test >/dev/null

S5_CLOSURE_REHEARSAL_SKIP_PREFLIGHT_COMMANDS=1 \
S5_CLOSURE_REHEARSAL_RUN_ROOT="$TMPDIR/runs" \
    "$SCRIPT" --fast --keep-artifacts >"$TMPDIR/rehearsal.out"

LOG_PATH="$(awk -F'log=' '/S5 closure rehearsal PASS/ {print $2}' "$TMPDIR/rehearsal.out")"

python3 - "$LOG_PATH" <<'PY'
import json
import pathlib
import sys

log_path = pathlib.Path(sys.argv[1])
records = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines()]
assert records, "rehearsal log should not be empty"

stage_ids = {record["stage_id"] for record in records}
assert set(range(13)).issubset(stage_ids), stage_ids

for record in records:
    assert record["event_name"] == "s5_closure_flow", record
    assert record["schema_id"] == "s5_closure_rehearsal_log", record
    assert record["schema_id"] != "s5_closure_log", record
    assert isinstance(record["stage_id"], int), record
    assert record["stage_name"], record

# s5_closure_rehearsal_log is the script-owned envelope schema. It wraps the
# shared s5_closure_flow event name and must not be confused with gbf-train's
# Rust helper payload schema, s5_closure_log.
assert {record["event_name"] for record in records} == {"s5_closure_flow"}, records
assert {record["schema_id"] for record in records} == {"s5_closure_rehearsal_log"}, records

expected_sequence = [
    (0, "stage_started"),
    (0, "preflight_summary"),
    (1, "stage_started"),
    (1, "s5_preregistration"),
    (2, "stage_started"),
    (2, "state_transition"),
    (2, "state_transition"),
    (2, "state_transition"),
    (2, "state_transition"),
    (2, "state_transition"),
    (2, "state_transition"),
    (2, "pick_frontier_inputs_frozen"),
    (3, "stage_started"),
    (3, "fit_revalidation_scaffold"),
    (4, "stage_started"),
    (4, "pf3_diff_scaffold"),
    (5, "stage_started"),
    (5, "negative_control_sample"),
    (6, "stage_started"),
    (6, "harness_run_scaffold"),
    (7, "stage_started"),
    (7, "frontier_emitted"),
    (8, "stage_started"),
    (8, "outcome_dispatch"),
    (9, "stage_started"),
    (9, "falsifier_trigger_scaffold"),
    (9, "falsifier_trigger_scaffold"),
    (10, "stage_started"),
    (10, "replay_determinism_scaffold"),
    (11, "stage_started"),
    (11, "logging_overhead_gate_scaffold"),
    (12, "outcome_dispatch_summary"),
]
observed_sequence = [(record["stage_id"], record["event_kind"]) for record in records]
assert observed_sequence == expected_sequence, observed_sequence

pick_transitions = [
    record
    for record in records
    if record["stage_id"] == 2 and record["event_kind"] == "state_transition"
]
assert [
    (record["fields"]["variant"], record["fields"]["seed"], record["fields"]["from"], record["fields"]["to"])
    for record in pick_transitions
] == [
    ("BoundedKv", 0, "PickRunning", "PickEvalDone"),
    ("BoundedKv", 0, "PickEvalDone", "PickScoreDone"),
    ("L_FIX1", 0, "PickRunning", "PickEvalDone"),
    ("L_FIX1", 0, "PickEvalDone", "PickScoreDone"),
    ("L_MT4", 0, "PickRunning", "PickEvalDone"),
    ("L_MT4", 0, "PickEvalDone", "PickScoreDone"),
], pick_transitions

negative = [record for record in records if record["event_kind"] == "negative_control_sample"]
assert len(negative) == 1, negative
fields = negative[0]["fields"]
assert fields["shadow_compile_ok"] is False, fields
assert fields["diagnostic"], fields
assert fields["failure_stage"] == "RomWindowPlan", fields
assert fields["fixture_role"] == "canonical_rehearsal_validator_json", fields
assert fields["validated_by"] == "gbf-policy::shadow::validate_shr1_shadow_sample", fields

summary = records[-1]
assert summary["stage_id"] == 12, summary
assert summary["event_kind"] == "outcome_dispatch_summary", summary
PY

python3 - "$ROOT/fixtures/s5/shadow/broken_negative_control.sample.json" "$TMPDIR/bad_negative_control.json" <<'PY'
import json
import pathlib
import sys

fixture = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
sample = json.loads(fixture.read_text(encoding="utf-8"))
sample["shadow_compile_ok"] = True
sample["shadow_compile_skipped"] = "this bad fixture should be rejected by the Rust SHR-1 helper"
out.write_text(json.dumps(sample, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if S5_CLOSURE_REHEARSAL_SKIP_PREFLIGHT_COMMANDS=1 \
   S5_CLOSURE_REHEARSAL_RUN_ROOT="$TMPDIR/bad-runs" \
   S5_CLOSURE_REHEARSAL_NEGATIVE_CONTROL_FIXTURE="$TMPDIR/bad_negative_control.json" \
       "$SCRIPT" --fast --keep-artifacts >"$TMPDIR/bad.out" 2>"$TMPDIR/bad.err"; then
    echo "s5_closure_rehearsal.sh should fail when the negative-control fixture reports shadow_compile_ok=true" >&2
    exit 1
fi

grep -F "broken SHR-1 sample must have ok=false" "$TMPDIR/bad.err" >/dev/null
grep -F "stage=5:negative_control_sample" "$TMPDIR/bad.err" >/dev/null

echo "[S5 CLOSURE REHEARSAL TEST] script checks passed"
