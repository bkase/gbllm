#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

log_dir="${GBF_E2E_LOG_DIR:-target/e2e-logs}"
log_file="$log_dir/quant_graph_fixtures.jsonl"
mkdir -p "$log_dir"
: > "$log_file"

log_event() {
  local event="$1"
  local status="$2"
  local fixture="${3:-}"
  local detail="${4:-}"
  printf '{"event":"%s","status":"%s","fixture":"%s","detail":"%s"}\n' \
    "$event" "$status" "$fixture" "$detail" >> "$log_file"
}

fail() {
  log_event "quant_graph_fixtures.error" "failed" "${1:-}" "${2:-}"
  printf 'quant_graph_fixtures: %s %s\n' "${1:-}" "${2:-}" >&2
  exit 1
}

require_file() {
  local path="$1"
  [[ -f "$path" ]] || fail "$path" "missing file"
}

passing_fixtures=(
  dense_toy0
  dense_toy1_tied
  dense_toy1_untied
  routed_basic_one
  routed_basic_selected_score
  mixed_topology
)

for fixture in "${passing_fixtures[@]}"; do
  dir="fixtures/quant_graph/$fixture"
  require_file "$dir/fixture.toml"
  require_file "$dir/quant_graph_self_hash"
  require_file "$dir/quant_graph_canonical_bytes_hash"
  require_file "$dir/report_self_hash"
  grep -Fq "name = \"$fixture\"" "$dir/fixture.toml" \
    || fail "$fixture" "fixture.toml name mismatch"
  log_event "quant_graph_fixture.pass_manifest" "ok" "$fixture" "hashes_present"
  log_event "quant_graph_fixture.cache_contract" "covered" "$fixture" "stage1_cache_hit_byte_identical"
done

reject_count=0
while IFS= read -r expected; do
  dir="$(dirname "$expected")"
  fixture="$(basename "$dir")"
  inputs="$dir/inputs.toml"
  readme="$dir/README.md"
  require_file "$inputs"
  require_file "$readme"

  qg_reject="$(sed -n 's/^qg_reject = //p' "$expected")"
  diagnostic_code="$(sed -n 's/^diagnostic_code = "\(.*\)"/\1/p' "$expected")"
  severity="$(sed -n 's/^severity = "\(.*\)"/\1/p' "$expected")"
  counterexample="$(sed -n 's/^counterexample = "\(.*\)"/\1/p' "$inputs")"

  [[ "$severity" == "Hard" ]] || fail "$fixture" "severity is not Hard"
  grep -Fq "qg_reject = $qg_reject" "$inputs" \
    || fail "$fixture" "inputs.toml qg_reject mismatch"
  grep -Fq "$diagnostic_code" "$readme" \
    || fail "$fixture" "README diagnostic missing"
  [[ "$counterexample" == gbf-codegen::s1::quant_graph::tests::* ]] \
    || fail "$fixture" "counterexample does not name Stage 1 test builder"
  reject_count=$((reject_count + 1))
  log_event "quant_graph_fixture.reject_manifest" "ok" "$fixture" "$diagnostic_code"
done < <(find fixtures/quant_graph/reject -mindepth 2 -maxdepth 2 -name expected.toml | sort)

[[ "$reject_count" -eq 36 ]] || fail "reject_count" "expected 36 got $reject_count"

log_event "quant_graph_fixture.cargo_test" "start" "fixture_quant_graph" "s1"
cargo test -p gbf-codegen --lib s1::quant_graph::tests::fixture_quant_graph -- --test-threads=1
log_event "quant_graph_fixture.cargo_test" "ok" "fixture_quant_graph" "s1"
