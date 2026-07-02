#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
report_path="$repo_root/experiments/S7/smoke/S7-smoke-report.md"

# This driver prints the committed deterministic smoke report. It does not
# post-process tracing logs; set S7_SMOKE_RUN_TESTS=1 to run the focused Rust
# smoke target before printing.
if [[ "${S7_SMOKE_RUN_TESTS:-0}" == "1" ]]; then
  cargo test -p gbf-experiments --features s7 --test integration_s7 e2e_s7_smoke -- --nocapture
fi

if [[ ! -f "$report_path" ]]; then
  printf 'missing committed smoke report: %s\n' "$report_path" >&2
  printf 'run: GBF_UPDATE_GOLDENS=1 cargo test -p gbf-experiments --features s7 --test integration_s7 e2e_s7_smoke_pass_matches_committed_outputs_and_closure_envelope\n' >&2
  exit 1
fi

cat "$report_path"
