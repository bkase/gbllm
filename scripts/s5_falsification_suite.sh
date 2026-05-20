#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_PATH="${S5_FALSIFICATION_SUITE_REPORT:-/tmp/s5-falsification-suite.json}"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_falsification_suite.sh [--self-test] [--report-path PATH]

RFC §18.10 entrypoint for the F-S5 falsification suite. This bounded wrapper
checks the committed policy falsifier substrate and the dry-run feature matrix.
It is SUBSTRATE_ONLY: it does not iterate live gbf-experiments::s5 producer
s5-falsify-N features. Live F1..F15 producer-loop execution is owned by bd-q3zo.
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
  "$0" --help | grep -F "SUBSTRATE_ONLY" >/dev/null
  "$0" --help | grep -F "Live F1..F15 producer-loop execution is owned by bd-q3zo" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f13" "$0" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f14" "$0" >/dev/null
  grep -F "cargo test -p gbf-policy --lib s5::tests::f15" "$0" >/dev/null
  grep -F "f13_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  grep -F "f14_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  grep -F "f15_" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  echo "[S5 FALSIFICATION SUITE] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --lib s5::tests::f13
cargo test -p gbf-policy --lib s5::tests::f14
cargo test -p gbf-policy --lib s5::tests::f15
"$ROOT/scripts/s5_feature_matrix_check.sh" --dry-run --report-path "$REPORT_PATH"

cat <<NOTE
S5 falsification suite PASS substrate=gbf-policy F13/F14/F15 + dry-run feature matrix
report=$REPORT_PATH
SUBSTRATE_ONLY: live gbf-experiments::s5 F1..F15 s5-falsify-N producer execution is not invoked here yet.
owner: bd-q3zo.
NOTE
