#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

SCRIPTS=(
  check-nucleus-drift.sh
  s5_predictions_ancestry.sh
  s5_falsification_suite.sh
  s5_reproducibility_smoke.sh
  s5_pareto_fixture_check.sh
  s5_feedback_fixture_check.sh
  s5_attention_oracle_fixture_check.sh
  s5_encoded_rom_cert_check.sh
  s5_emulator_harness_seed0_check.sh
)

for script in "${SCRIPTS[@]}"; do
  path="$ROOT/scripts/$script"
  test -x "$path"
  bash -n "$path"
  "$path" --self-test >"$TMPDIR/${script}.self-test.out"
  grep -F "PASS" "$TMPDIR/${script}.self-test.out" >/dev/null
done

head_commit="$(git -C "$ROOT" rev-parse HEAD)"
cat >"$TMPDIR/s5_report.json" <<JSON
{
  "schema": "s5_report.v1",
  "predictions_commit": "$head_commit",
  "first_result_commit": "$head_commit"
}
JSON

"$ROOT/scripts/s5_predictions_ancestry.sh" \
  --report "$TMPDIR/s5_report.json" \
  >"$TMPDIR/predictions-ancestry.out"
grep -F "S5 predictions ancestry PASS" "$TMPDIR/predictions-ancestry.out" >/dev/null

"$ROOT/scripts/s5_predictions_ancestry.sh" >"$TMPDIR/predictions-substrate.out"
grep -F "SUBSTRATE_ONLY" "$TMPDIR/predictions-substrate.out" >/dev/null

"$ROOT/scripts/s5_reproducibility_smoke.sh" >"$TMPDIR/reproducibility.out"
grep -F "S5 reproducibility smoke PASS" "$TMPDIR/reproducibility.out" >/dev/null
grep -F "SUBSTRATE_ONLY" "$TMPDIR/reproducibility.out" >/dev/null

"$ROOT/scripts/s5_falsification_suite.sh" \
  --report-path "$TMPDIR/falsification-suite.json" \
  >"$TMPDIR/falsification.out"
grep -F "S5 falsification suite PASS" "$TMPDIR/falsification.out" >/dev/null
grep -F "live gbf-experiments::s5 F1..F15 feature loop" "$TMPDIR/falsification.out" >/dev/null
grep -F "LIMITATION: full S5 producer replay APIs are not implemented" "$TMPDIR/falsification.out" >/dev/null
test -f "$TMPDIR/falsification-suite.json"

echo "[S5 RFC GATE WRAPPERS TEST] script checks passed"
