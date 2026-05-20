#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_PATH="${S5_FALSIFICATION_SUITE_REPORT:-/tmp/s5-falsification-suite.json}"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_falsification_suite.sh [--self-test] [--report-path PATH]

RFC §18.10 entrypoint for the F-S5 falsification suite. This bounded wrapper
checks the committed policy falsifier substrate, then iterates live
gbf-experiments::s5 falsification cases one feature at a time for
s5-falsify-1 through s5-falsify-15.

Honesty note: upstream full S5 producer replay APIs are not implemented yet.
The gbf-experiments::s5 harness therefore runs explicit producer-contract
fixtures for cases that cannot replay closure artifacts, and records that
limitation in each case report.
USAGE
}

self_test=0
while (($#)); do
  case "$1" in
    --report-path)
      shift
      if (($# == 0)); then
        echo "s5_falsification_suite.sh: --report-path requires a path" >&2
        exit 2
      fi
      REPORT_PATH="$1"
      ;;
    --self-test)
      self_test=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "s5_falsification_suite.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  "$0" --help | grep -F "s5-falsify-1 through s5-falsify-15" >/dev/null
  "$0" --help | grep -F "producer-contract" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f13" "$0" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f14" "$0" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f15" "$0" >/dev/null
  grep -F "cargo run -q -p gbf-experiments --bin s5_falsification_loop" "$0" >/dev/null
  grep -F "active_s5_falsify_feature_refutes_its_target" "$ROOT/gbf-experiments/tests/s5_falsification.rs" >/dev/null
  grep -F "S5_EXPLICIT_FIXTURE_LIMITATION" "$ROOT/gbf-experiments/src/s5/falsify.rs" >/dev/null
  grep -F "run_active_s5_falsification_case" "$ROOT/gbf-experiments/src/bin/s5_falsification_loop.rs" >/dev/null
  grep -F "f13_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  grep -F "f14_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  grep -F "f15_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  echo "[S5 FALSIFICATION SUITE] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --lib s5::tests::f13
cargo test -p gbf-policy --lib s5::tests::f14
cargo test -p gbf-policy --lib s5::tests::f15

case_dir="$(mktemp -d "${TMPDIR:-/tmp}/s5-falsification-cases.XXXXXX")"
cleanup() {
  rm -rf "$case_dir"
}
trap cleanup EXIT

case_reports=()
for n in $(seq 1 15); do
  case_report="$case_dir/s5-falsify-$n.json"
  features="s5-default,qat,burn-adapter,s5-falsify-$n"
  echo "S5 falsification live case start feature=s5-falsify-$n"
  cargo run -q -p gbf-experiments \
      --bin s5_falsification_loop \
      --no-default-features \
      --features "$features" >"$case_report"
  test -f "$case_report"
  case_reports+=("$case_report")
done

"$ROOT/scripts/s5_feature_matrix_check.sh" --dry-run --report-path "$case_dir/feature-matrix.json"

mkdir -p "$(dirname "$REPORT_PATH")"
python3 - "$REPORT_PATH" "$case_dir/feature-matrix.json" "${case_reports[@]}" <<'PY'
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
feature_matrix_path = Path(sys.argv[2])
case_paths = [Path(path) for path in sys.argv[3:]]
cases = [json.loads(path.read_text(encoding="utf-8")) for path in case_paths]
report = {
    "schema": "s5_falsification_suite.v1",
    "script": "s5_falsification_suite",
    "policy_substrate": ["f13", "f14", "f15"],
    "feature_matrix_report": str(feature_matrix_path),
    "case_count": len(cases),
    "cases": cases,
    "passed": len(cases) == 15 and all(case["matches_expected"] for case in cases),
    "limitation": (
        "full S5 producer replay APIs are not implemented; gbf-experiments::s5 "
        "runs explicit producer-contract fixtures where replay inputs do not exist"
    ),
}
report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
if not report["passed"]:
    raise SystemExit("S5 falsification suite failed")
PY

cat <<NOTE
S5 falsification suite PASS substrate=gbf-policy F13/F14/F15 + live gbf-experiments::s5 F1..F15 feature loop + dry-run feature matrix
report=$REPORT_PATH
LIMITATION: full S5 producer replay APIs are not implemented; explicit gbf-experiments::s5 producer-contract fixtures are used where replay inputs do not exist.
NOTE
