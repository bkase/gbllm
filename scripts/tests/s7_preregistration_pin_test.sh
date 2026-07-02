#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PIN_SCRIPT="$ROOT/scripts/s7_preregistration_pin.sh"
CHECK_SCRIPT="$ROOT/scripts/s7_preregistration_check.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

init_repo() {
  git init -q "$TMPDIR/repo"
  cd "$TMPDIR/repo"
  git config user.email "s7-pin@example.test"
  git config user.name "S7 Pin Test"
}

write_rfc() {
  local mutation="${1:-}"
  mkdir -p history/rfcs
  python3 - "$mutation" <<'PY'
import sys
from pathlib import Path

mutation = sys.argv[1]
lines = [
    "# F-S7 fixture",
    "",
    "# 1. Hypothesis algebra",
    "",
    "```text",
    "H1: MoeTiny completes all seeds.",
    "H2: Dense matched completes all seeds.",
    "H3: matched bytes remain within tolerance.",
    "H4: MoE dominates dense on median bpc.",
    "H5: switch stats remain healthy.",
    "H6: lambda_switch sweep does not collapse.",
    "H7: gradients reach only declared tensors.",
    "H8: ExpertBlockQat burn smoke passes.",
    "H9: routed oracle agrees.",
    "H10: emulator one-token passes.",
    "Decision: ProceedToS8 if all gates confirm.",
    "```",
    "",
    "# 2. Authority rules",
]
if mutation:
    lines[8] = f"H4: MoE dominates dense on median bpc {mutation}."
Path("history/rfcs/F-S7-moe-beats-dense.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
}

expect_fail() {
  local label="$1"
  local expected="$2"
  set +e
  "$PIN_SCRIPT" --pass-version s7-prereg-test-2026-06-25 --output - >"$TMPDIR/$label.out" 2>"$TMPDIR/$label.err"
  local status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "expected $label to fail" >&2
    cat "$TMPDIR/$label.out" >&2
    exit 1
  fi
  grep -F "$expected" "$TMPDIR/$label.err" >/dev/null
}

init_repo
write_rfc
git add history/rfcs/F-S7-moe-beats-dense.md
git commit -q -m "pre-register S7 predictions"
predictions_commit="$(git rev-parse HEAD)"

"$PIN_SCRIPT" --pass-version s7-prereg-test-2026-06-25 --output fixtures/preregistration/s7.toml >/tmp/s7-preregistration-pin.out
test -f fixtures/preregistration/s7.toml
grep -F 'schema = "s7_preregistration.v1"' fixtures/preregistration/s7.toml >/dev/null
grep -F 'predictions_line_start = 3' fixtures/preregistration/s7.toml >/dev/null
grep -F 'predictions_line_end = 17' fixtures/preregistration/s7.toml >/dev/null
grep -F "predictions_commit = \"$predictions_commit\"" fixtures/preregistration/s7.toml >/dev/null
"$CHECK_SCRIPT" >/dev/null

"$PIN_SCRIPT" --check-ready >"$TMPDIR/check_ready.json"
python3 - "$TMPDIR/check_ready.json" "$predictions_commit" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert payload["script"] == "s7_preregistration_pin"
assert payload["ready"] is True
assert payload["line_range"] == "3..17"
assert payload["predictions_commit"] == sys.argv[2]
assert payload["rfc_path"] == "history/rfcs/F-S7-moe-beats-dense.md"
assert payload["predictions_section_hash"].startswith("sha256:")
PY

write_rfc "MUTATED"
expect_fail dirty_rfc "current RFC predictions section differs from predictions_commit"

write_rfc
python3 - <<'PY'
from pathlib import Path

path = Path("history/rfcs/F-S7-moe-beats-dense.md")
text = path.read_text(encoding="utf-8")
path.write_text("prepended context line\n" + text, encoding="utf-8")
PY
expect_fail line_drift "current RFC prediction heading line range differs from predictions_commit"
set +e
"$PIN_SCRIPT" --check-ready >"$TMPDIR/check_ready_line_drift.out" 2>"$TMPDIR/check_ready_line_drift.err"
status=$?
set -e
if [[ "$status" -eq 0 ]]; then
  echo "expected check-ready line drift to fail" >&2
  exit 1
fi
grep -F "current RFC prediction heading line range differs from predictions_commit" \
  "$TMPDIR/check_ready_line_drift.err" >/dev/null

write_rfc
set +e
"$PIN_SCRIPT" --pass-version fixture --output - >"$TMPDIR/placeholder_pass.out" 2>"$TMPDIR/placeholder_pass.err"
status=$?
set -e
if [[ "$status" -eq 0 ]]; then
  echo "expected placeholder pass_version to fail" >&2
  exit 1
fi
grep -F "pass_version_S7 must be finalized" "$TMPDIR/placeholder_pass.err" >/dev/null

side_commit="$(git commit-tree HEAD^{tree} -m "side prereg commit")"
set +e
"$PIN_SCRIPT" \
  --pass-version s7-prereg-test-2026-06-25 \
  --predictions-commit "$side_commit" \
  --output - >"$TMPDIR/side_predictions.out" 2>"$TMPDIR/side_predictions.err"
status=$?
set -e
if [[ "$status" -eq 0 ]]; then
  echo "expected side-branch predictions_commit to fail" >&2
  exit 1
fi
grep -F "predictions_commit must be an ancestor of HEAD/current checkout" "$TMPDIR/side_predictions.err" >/dev/null

set +e
"$PIN_SCRIPT" \
  --pass-version s7-prereg-test-2026-06-25 \
  --rfc-revision "$side_commit" \
  --output - >"$TMPDIR/side_rfc.out" 2>"$TMPDIR/side_rfc.err"
status=$?
set -e
if [[ "$status" -eq 0 ]]; then
  echo "expected side-branch rfc_revision to fail" >&2
  exit 1
fi
grep -F "rfc_revision must be an ancestor of HEAD/current checkout" "$TMPDIR/side_rfc.err" >/dev/null

set +e
"$PIN_SCRIPT" \
  --pass-version s7-prereg-test-2026-06-25 \
  --first-result-commit "$predictions_commit" \
  --output - >"$TMPDIR/same_result.out" 2>"$TMPDIR/same_result.err"
status=$?
set -e
if [[ "$status" -eq 0 ]]; then
  echo "expected non-strict first_result_commit to fail" >&2
  exit 1
fi
grep -F "predictions_commit must be a strict ancestor of first_result_commit" "$TMPDIR/same_result.err" >/dev/null

echo "s7_preregistration_pin_test: ok"
