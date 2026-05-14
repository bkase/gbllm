#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# shellcheck disable=SC1091
source scripts/e2e/lib/chunk2_logging.sh

timestamp="$(date -u '+%Y%m%dT%H%M%SZ')"
log_dir="${CHUNK2_E2E_LOG_DIR:-target/e2e-logs}"
log_file="$log_dir/chunk2_reject.${timestamp}.$$.jsonl"
verbose="${CHUNK2_E2E_VERBOSE:-0}"
chunk2_log_init "$log_file"

if [[ "$verbose" == "1" ]]; then
  chunk2_log_event "$log_file" "chunk2.reject.debug" \
    fixture chunk2 level verbose detail "verbose logging enabled"
fi

fail() {
  local fixture="${1:-chunk2}"
  local expected="${2:-unknown}"
  local observed="${3:-failed}"
  chunk2_log_event "$log_file" "chunk2.reject.diff" \
    fixture "$fixture" expected_class "$expected" observed_class "$observed" severity "" message_excerpt "$observed"
  printf 'chunk2_reject: %s expected %s observed %s\n' "$fixture" "$expected" "$observed" >&2
  exit 1
}

check_expected_file() {
  local family="$1"
  local expected_file="$2"
  local fixture
  fixture="$(basename "$(dirname "$expected_file")")"
  local code severity reject_id
  if [[ "$family" == "qg" ]]; then
    reject_id="$(sed -n 's/^qg_reject = //p' "$expected_file")"
    code="$(chunk2_read_toml_string diagnostic_code "$expected_file")"
    severity="$(chunk2_read_toml_string severity "$expected_file")"
  else
    reject_id="$(sed -n 's/^reject_id = //p' "$expected_file")"
    code="$(chunk2_read_toml_string code "$expected_file")"
    severity="$(chunk2_read_toml_string severity "$expected_file")"
  fi

  [[ -n "$reject_id" ]] || fail "$fixture" "$family" "missing reject id"
  [[ -n "$code" ]] || fail "$fixture" "$family" "missing typed diagnostic code"
  [[ "$severity" == "Hard" ]] || fail "$fixture" "$code" "severity=$severity"
  [[ "$code" != *String* ]] || fail "$fixture" "$code" "generic string diagnostic"

  chunk2_log_event "$log_file" "chunk2.reject.expected_class" \
    fixture "$fixture" family "$family" expected_class "$code" expected_source "$expected_file"
}

run_gate() {
  local gate="$1"
  shift
  chunk2_log_event "$log_file" "chunk2.reject.gate.start" fixture chunk2 gate "$gate"
  "$@" || fail chunk2 "$gate" "gate failed"
  chunk2_log_event "$log_file" "chunk2.reject.gate.complete" \
    fixture chunk2 gate "$gate" status passed
}

mapfile -t qg_expected < <(find fixtures/quant_graph/reject -mindepth 2 -maxdepth 2 -name expected.toml | sort)
mapfile -t iir_expected < <(find fixtures/infer_ir/reject -mindepth 2 -maxdepth 2 -name expected.toml | sort)

[[ "${#qg_expected[@]}" -eq 36 ]] || fail quant_graph 36 "count=${#qg_expected[@]}"
[[ "${#iir_expected[@]}" -eq 36 ]] || fail infer_ir 36 "count=${#iir_expected[@]}"

run_gate infer_ir_reject_taxonomy \
  cargo test -p gbf-codegen --lib fixture_infer_ir_every_reject_class_typed_diagnostic -- --test-threads=1
run_gate quant_graph_reject_taxonomy \
  cargo test -p gbf-codegen --lib s1::quant_graph::tests::fixture_quant_graph_every_reject_class_has_typed_diagnostic -- --test-threads=1

for expected in "${qg_expected[@]}"; do
  check_expected_file qg "$expected"
done

for expected in "${iir_expected[@]}"; do
  check_expected_file iir "$expected"
done

chunk2_log_event "$log_file" "chunk2.reject.complete" \
  fixture chunk2 total_ms 0 status verified expected_class_count 72 executable_gates_passed true

printf '%s\n' "$log_file"
