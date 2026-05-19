#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

fixture="${1:-chunked_i16}"
build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-stage5}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
fixture_dir="gbf-codegen/tests/fixtures/f_b6_f_b7/accept/stage5/$fixture"
report_dir="$out_dir/reports/stage5/$fixture"
ndjson="$out_dir/stage5-run.ndjson"

case "$fixture" in
  single_i16|chunked_i16|renorm_loop_non_bitexact|ceiling_override_layer_site) ;;
  *)
    echo "error: unknown Stage 5 accept fixture '$fixture'" >&2
    exit 2
    ;;
esac

if [[ ! -d "$fixture_dir" ]]; then
  echo "error: missing Stage 5 fixture directory $fixture_dir" >&2
  exit 1
fi

mkdir -p "$report_dir"

export F_B6_F_B7_BUILD_ID="$build_id"
export F_B6_F_B7_OUT_DIR="$out_dir"
export F_B6_F_B7_STAGE5_FIXTURE="$fixture"

cargo test -p gbf-codegen \
  's5::range_plan::tests::stage5_script_harness_runs_real_driver_fixture' \
  --lib -- --exact --nocapture

if [[ ! -s "$ndjson" ]]; then
  echo "error: Stage 5 real-driver harness did not write $ndjson" >&2
  exit 1
fi

if [[ ! -f "$report_dir/range_plan.json" ]]; then
  echo "error: Stage 5 real-driver harness did not write $report_dir/range_plan.json" >&2
  exit 1
fi

if [[ ! -f "$report_dir/certs/range.cert.json" ]]; then
  echo "error: Stage 5 real-driver harness did not write $report_dir/certs/range.cert.json" >&2
  exit 1
fi

echo "stage5 real-driver run complete: fixture=$fixture out_dir=$out_dir"
