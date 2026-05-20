#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

assert_no_tmp_s4_reports() {
  local workflow="$1"
  if grep -F "/tmp/s4-" "$ROOT/.github/workflows/$workflow" >/dev/null; then
    echo "$workflow must write uploaded S4 JSON reports under artifacts/, not /tmp" >&2
    return 1
  fi
}

SCRIPTS=(
  "s4_preregistration_check.sh"
  "s4_determinism_check.sh"
  "s4_full_determinism_check.sh"
  "s4_isolation_check.sh"
  "s4_api_drift_check.sh"
)

for script in "${SCRIPTS[@]}"; do
  bash -n "$ROOT/scripts/$script"
done
python3 -m py_compile "$ROOT/scripts/s4_ci_common.py"

"$ROOT/scripts/s4_preregistration_check.sh" \
  --dry-run \
  --output "$TMPDIR/s4-preregistration.json" \
  >/dev/null

for script in \
  s4_determinism_check.sh \
  s4_full_determinism_check.sh \
  s4_isolation_check.sh \
  s4_api_drift_check.sh
do
  "$ROOT/scripts/$script" --dry-run --report-dir "$TMPDIR" >/dev/null
done

"$ROOT/scripts/s4_determinism_check.sh" \
  --report-path "$TMPDIR/s4-determinism-live.json" \
  >/dev/null

mkdir -p "$TMPDIR/replay-a/seed-0" "$TMPDIR/replay-b/seed-0"
cat >"$TMPDIR/replay-a/seed-0/checkpoint.metadata.json" <<'JSON'
{"schema":"s4_gutenberg_checkpoint.v1","seed":0,"catalog_snapshot_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","gutenberg_manifest_self_hash":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","source_blob_sha256":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","build_kind":"phase_d_continuation","device_profile":"S1CpuDeterministic","checkpoint_self_hash":"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}
JSON
cp "$TMPDIR/replay-a/seed-0/checkpoint.metadata.json" \
  "$TMPDIR/replay-b/seed-0/checkpoint.metadata.json"

"$ROOT/scripts/s4_determinism_check.sh" \
  --artifact-dir "$TMPDIR/replay-a" \
  --artifact-dir "$TMPDIR/replay-b" \
  --report-path "$TMPDIR/s4-determinism-pair.json" \
  >/dev/null

python3 - "$TMPDIR/replay-b/seed-0/checkpoint.metadata.json" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
path.write_text(
    path.read_text(encoding="utf-8").replace(
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    ),
    encoding="utf-8",
)
PY

if "$ROOT/scripts/s4_determinism_check.sh" \
  --artifact-dir "$TMPDIR/replay-a" \
  --artifact-dir "$TMPDIR/replay-b" \
  --report-path "$TMPDIR/s4-determinism-pair-fail.json" \
  >/dev/null 2>/dev/null; then
  echo "s4_determinism_check.sh should fail when replay artifact bytes differ" >&2
  exit 1
fi

python3 - "$TMPDIR" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected = {
    "s4-preregistration.json": "s4_preregistration_check",
    "s4-determinism.json": "s4_determinism_check",
    "s4-full-determinism.json": "s4_full_determinism_check",
    "s4-isolation.json": "s4_isolation_check",
    "s4-api-drift.json": "s4_api_drift_check",
}
for filename, script in expected.items():
    report = json.loads((root / filename).read_text(encoding="utf-8"))
    assert report["script"] == script, report
    assert report["passed"] is True, report
    assert report["exit_code"] == 0, report
    if script == "s4_preregistration_check":
        assert report["dry_run"] is True, report
        event_names = [event["event"] for event in report["events"]]
        assert "s4_prereg_check_passed" in event_names, report
        stage_names = [
            event["name"]
            for event in report["events"]
            if event["event"] == "s4_prereg_stage_done"
        ]
        assert "predictions_hash" in stage_names, report
        assert "pin_history" in stage_names, report
        assert any(name in stage_names for name in ("pre_result_scan", "first_result_ordering")), report
    else:
        assert report["dry_run"] is True, report
        assert report["evidence_mode"] == "dry_run", report
        assert report["live_evidence"] is False, report
        assert report["stages"], report
        if script == "s4_api_drift_check":
            surface = next(stage for stage in report["stages"] if stage["name"] == "s4_public_module_surface")
            assert "optional" in surface["detail"], report

live = json.loads((root / "s4-determinism-live.json").read_text(encoding="utf-8"))
assert live["script"] == "s4_determinism_check", live
assert live["passed"] is True, live
assert live["dry_run"] is False, live
assert live["live_evidence"] is True, live
assert live["stages"][0]["name"] == "d16_replay_inputs", live
assert live["stages"][0]["detail"]["gutenberg_fixture"]["checked_source_blob_count"] == 8, live
assert live["stages"][-1]["detail"]["matched"] is True, live

pair = json.loads((root / "s4-determinism-pair.json").read_text(encoding="utf-8"))
assert pair["passed"] is True, pair
assert pair["stages"][-1]["detail"]["mode"] == "artifact_pair_replay", pair
assert pair["stages"][1]["detail"]["hashes"]["catalog_snapshot_sha256"].endswith("a" * 64), pair
assert pair["stages"][1]["detail"]["hashes"]["gutenberg_manifest_self_hash"].endswith("b" * 64), pair

failed_pair = json.loads((root / "s4-determinism-pair-fail.json").read_text(encoding="utf-8"))
assert failed_pair["passed"] is False, failed_pair
failed_stage = failed_pair["stages"][-1]
assert failed_stage["name"] == "bytewise_compare", failed_pair
assert failed_stage["passed"] is False, failed_pair
assert failed_stage["detail"]["diff"]["artifact_changed_count"] == 1, failed_pair
PY

for workflow in \
  s4-preregistration.yml \
  s4-determinism.yml \
  s4-pr.yml \
  s4-nightly.yml \
  s4-on-demand.yml
do
  test -f "$ROOT/.github/workflows/$workflow"
  grep -F "actions/checkout@v4" "$ROOT/.github/workflows/$workflow" >/dev/null
  grep -F "fetch-depth: 0" "$ROOT/.github/workflows/$workflow" >/dev/null
  grep -F "actions/upload-artifact@v4" "$ROOT/.github/workflows/$workflow" >/dev/null
done

grep -F '"experiments/S4/**"' "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F '"docs/experiments/S4-report.md"' "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F '"gbf-experiments/src/s4/**"' "$ROOT/.github/workflows/s4-determinism.yml" >/dev/null
grep -F "scripts/s4_determinism_check.sh --report-path" "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F "scripts/s4_full_determinism_check.sh --dry-run" "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F "scripts/s4_isolation_check.sh --dry-run" "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F "scripts/s4_api_drift_check.sh --dry-run" "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F "pre-result dry-run" "$ROOT/.github/workflows/s4-preregistration.yml" >/dev/null
grep -F "determinism smoke" "$ROOT/.github/workflows/s4-determinism.yml" >/dev/null
grep -F "determinism smoke" "$ROOT/.github/workflows/s4-pr.yml" >/dev/null
grep -F "determinism smoke and dry-run scaffolding" "$ROOT/.github/workflows/s4-nightly.yml" >/dev/null
grep -F "determinism smoke and dry-run scaffolding" "$ROOT/.github/workflows/s4-on-demand.yml" >/dev/null
assert_no_tmp_s4_reports s4-preregistration.yml
assert_no_tmp_s4_reports s4-determinism.yml
assert_no_tmp_s4_reports s4-pr.yml
assert_no_tmp_s4_reports s4-nightly.yml
assert_no_tmp_s4_reports s4-on-demand.yml

echo "[S4 CI TEST] all S4 CI script/workflow checks passed"
