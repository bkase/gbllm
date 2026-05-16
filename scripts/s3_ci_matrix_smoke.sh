#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-ci-matrix.XXXXXX")"
EVENTS="/tmp/s3-ci-matrix.json"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"
: > "$EVENTS"

scripts=(
  s3_preregistration_check
  s3_determinism_check
  s3_full_determinism_check
  s3_isolation_check
  s3_api_drift_check
  s3_oracle_re_run_check
  s3_no_naming_resolution_check
  s3_feature_matrix_check
)

for script in "${scripts[@]}"; do
  "scripts/${script}.sh" --dry-run --report-dir "$RUN_DIR/reports" \
    > "$RUN_DIR/${script}.stdout" \
    2> "$RUN_DIR/${script}.ndjson"
  cat "$RUN_DIR/${script}.ndjson" >> "$EVENTS"
done

python3 - "$EVENTS" "${scripts[@]}" <<'PY'
import json
import sys
from pathlib import Path

events = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line.strip()]
scripts = sys.argv[2:]
for script in scripts:
    names = [event["event"] for event in events if event.get("script") == script or event.get("event", "").startswith(script)]
    assert f"{script}_stage_start" in names, (script, names)
    assert f"{script}_stage_done" in names, (script, names)
    assert f"{script}_summary" in names, (script, names)
    summary = [event for event in events if event.get("event") == f"{script}_summary"][-1]
    assert summary["passed"] is True, summary
    assert summary["exit_code"] == 0, summary
    assert summary["dry_run"] is True, summary
PY

printf 'S3 CI matrix smoke PASS events=%s reports=%s\n' "$EVENTS" "$RUN_DIR/reports"
