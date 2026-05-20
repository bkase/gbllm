#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s5-feature-matrix-test.XXXXXX")"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

REPORT="$RUN_DIR/report.json"
STDOUT="$RUN_DIR/stdout.txt"
STDERR="$RUN_DIR/stderr.ndjson"

scripts/s5_feature_matrix_check.sh --dry-run --report-path "$REPORT" >"$STDOUT" 2>"$STDERR"

grep -F 'S5 feature matrix PASS dry_run=1 sample=0 rows=17' "$STDOUT" >/dev/null
grep -F 's5-default,qat,burn-adapter' "$STDERR" >/dev/null
grep -F 's5-no-log,qat,burn-adapter' "$STDERR" >/dev/null
grep -F 's5-default,qat,burn-adapter,s5-falsify-1' "$STDERR" >/dev/null
grep -F 's5-default,qat,burn-adapter,s5-falsify-14' "$STDERR" >/dev/null
grep -F 's5-default,qat,burn-adapter,s5-falsify-15' "$STDERR" >/dev/null
grep -F 'S5 feature mutex violated: s5-default and s5-no-log are mutually exclusive' "$STDERR" >/dev/null
grep -F 'S5 falsifier feature mutex violated: enable at most one s5-falsify-N feature' "$STDERR" >/dev/null

python3 - "$REPORT" <<'PY'
import json
import sys
from pathlib import Path

report = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert report["script"] == "s5_feature_matrix_check", report
assert report["dry_run"] is True, report
assert report["passed"] is True, report
assert len(report["rows"]) == 17, report
features = {row["features"] for row in report["rows"]}
assert "s5-default,qat,burn-adapter" in features
assert "s5-no-log,qat,burn-adapter" in features
for n in range(1, 16):
    assert f"s5-default,qat,burn-adapter,s5-falsify-{n}" in features
assert {row["name"] for row in report["mutex_checks"]} == {
    "s5-default-vs-s5-no-log",
    "s5-falsify-pair",
}
PY

printf 'S5 feature matrix script test PASS report=%s\n' "$REPORT"
