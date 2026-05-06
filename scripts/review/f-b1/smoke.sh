#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

export RUST_LOG="${RUST_LOG:-info,gbf=debug}"
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

start="$(date +%s)"

run_block() {
  local label="$1"
  shift
  echo
  echo "::: $label"
  "$@"
}

run_block "F-B1.0 SquareDim" cargo test -p gbf-abi -- compute_shape::
run_block "F-B1.0 verifier matmul" cargo test -p gbf-verify -- matmul::
run_block "F-B1.0 codegen foundations" cargo test -p gbf-codegen -- quarter_square_
run_block "F-B1.0 dependency direction" cargo test -p gbf-meta-checks -- production_crates_do_not_depend_on_gbf_verify
run_block "F-B1.6 ignored heavy gates" cargo test -p gbf-meta-checks -- ignored_discipline_heavy_f_b1_tests_are_ignored
run_block "F-B1.0 tracing vocabulary" cargo test -p gbf-runtime -- trace::
run_block "F-B1.1-L2 emu fast gates" cargo test -p gbf-emu -- f_b1_l
run_block "F-B1.3-L4 runtime fast gates" cargo test -p gbf-runtime -- f_b1
run_block "F-B1.6 report schema" cargo test -p gbf-report -- realism_report_v1
run_block "F-B1.6 bench aggregation" cargo test -p gbf-bench -- f_b1

end="$(date +%s)"
echo
echo "F-B1 smoke OK in $((end - start))s. Heavy gates: scripts/review/f-b1/regen.sh"
