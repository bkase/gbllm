#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# shellcheck disable=SC1091
source scripts/e2e/lib/chunk2_logging.sh

timestamp="$(date -u '+%Y%m%dT%H%M%SZ')"
log_dir="${CHUNK2_E2E_LOG_DIR:-target/e2e-logs}"
log_file="$log_dir/chunk2_pipeline.${timestamp}.$$.jsonl"
f_b3_golden="${CHUNK2_E2E_F_B3_GOLDEN_DIR:-docs/review/f-b3/golden}"
f_b5_golden="${CHUNK2_E2E_F_B5_GOLDEN_DIR:-docs/review/f-b5/golden}"
skip_gates="${CHUNK2_E2E_SKIP_GATES:-0}"
verbose="${CHUNK2_E2E_VERBOSE:-0}"
chunk2_log_init "$log_file"

if [[ "$verbose" == "1" ]]; then
  chunk2_log_event "$log_file" "chunk2.pipeline.debug" \
    fixture chunk2 level verbose detail "verbose logging enabled"
fi

fail() {
  local fixture="${1:-chunk2}"
  local detail="${2:-failed}"
  chunk2_log_event "$log_file" "chunk2.pipeline.complete" \
    fixture "$fixture" total_ms 0 all_stages_passed false status failed detail "$detail"
  printf 'chunk2_pipeline: %s: %s\n' "$fixture" "$detail" >&2
  exit 1
}

run_gate() {
  local fixture="$1"
  local gate="$2"
  shift 2
  if [[ "$skip_gates" == "1" ]]; then
    chunk2_log_event "$log_file" "chunk2.pipeline.gate.skipped" \
      fixture "$fixture" gate "$gate" reason CHUNK2_E2E_SKIP_GATES
    return 0
  fi
  chunk2_log_event "$log_file" "chunk2.pipeline.gate.start" fixture "$fixture" gate "$gate"
  "$@" || fail "$fixture" "gate failed: $gate"
  chunk2_log_event "$log_file" "chunk2.pipeline.gate.complete" \
    fixture "$fixture" gate "$gate" status passed
}

json_field() {
  local path="$1"
  local dotted="$2"
  python3 - "$path" "$dotted" <<'PY'
import json
import sys

value = json.loads(open(sys.argv[1], encoding="utf-8").read())
for key in sys.argv[2].split("."):
    value = value[key]
print(value)
PY
}

toml_value() {
  local path="$1"
  local key="$2"
  sed -n "s/^${key} = \"\\(.*\\)\"/\\1/p" "$path"
}

mapfile -t qg_fixtures < <(find fixtures/quant_graph -mindepth 1 -maxdepth 1 -type d ! -name reject -exec basename {} \; | sort)
mapfile -t iir_fixtures < <(find fixtures/infer_ir -mindepth 1 -maxdepth 1 -type d ! -name reject -exec basename {} \; | sort)

[[ "${#qg_fixtures[@]}" -gt 0 ]] || fail "quant_graph" "no passing fixtures found"
[[ "${#iir_fixtures[@]}" -gt 0 ]] || fail "infer_ir" "no passing fixtures found"

run_gate chunk2 review_f_b3 scripts/review/f-b3/verify.sh
run_gate chunk2 review_f_b5 env GBF_REVIEW_F_B5_RUN_CARGO=1 scripts/review/f-b5/verify.sh
run_gate chunk2 quant_graph_fixtures scripts/e2e/quant_graph_fixtures.sh
run_gate chunk2 stage3 scripts/e2e/stage3.sh
run_gate chunk2 stage1_cache_hit_byte_identical \
  cargo test -p gbf-codegen --lib fixture_quant_graph_cache_hit_byte_identical_product -- --test-threads=1
run_gate chunk2 stage3_cache_hit_audit_rewrap \
  cargo test -p gbf-codegen --lib s3::infer_ir::tests::run_stage3_cache_hit_replays_with_audit_rewrap -- --test-threads=1
run_gate chunk2 stage3_semantic_equivalence_bit_exact \
  cargo test -p gbf-codegen --features semantic_equivalence_check --lib fixture_infer_ir_fixture_semantic_equivalence_bit_exact -- --test-threads=1

for fixture in "${qg_fixtures[@]}"; do
  start_ms="$(date +%s%3N)"
  chunk2_log_event "$log_file" "chunk2.pipeline.start" fixture "$fixture" profile quant_graph run_index 1
  golden_json="$f_b3_golden/pass/$fixture/quant_graph.json"
  [[ -f "$golden_json" ]] || {
    chunk2_log_event "$log_file" "chunk2.pipeline.golden.diff" \
      fixture "$fixture" stage quant_graph expected "$golden_json" observed missing diff_path ""
    fail "$fixture" "missing F-B3 quant_graph golden"
  }
  expected_qg_hash="$(tr -d '\n' < "fixtures/quant_graph/$fixture/quant_graph_self_hash")"
  observed_qg_hash="$(json_field "$golden_json" quant_graph_self_hash)"
  [[ "$observed_qg_hash" == "$expected_qg_hash" ]] || {
    chunk2_log_event "$log_file" "chunk2.pipeline.golden.diff" \
      fixture "$fixture" stage quant_graph expected "$expected_qg_hash" observed "$observed_qg_hash" diff_path "$golden_json"
    fail "$fixture" "quant_graph hash drift"
  }
  report_hash="$(tr -d '\n' < "fixtures/quant_graph/$fixture/report_self_hash")"
  golden_hash="$(chunk2_sha256_file "$golden_json")"
  chunk2_log_event "$log_file" "chunk2.pipeline.stage.complete" \
    fixture "$fixture" stage quant_graph report_self_hash "$report_hash"
  chunk2_log_event "$log_file" "chunk2.pipeline.golden.match" \
    fixture "$fixture" stage quant_graph golden_hash "$golden_hash"
  end_ms="$(date +%s%3N)"
  chunk2_log_event "$log_file" "chunk2.pipeline.complete" \
    fixture "$fixture" total_ms "$((end_ms - start_ms))" all_stages_passed true status verified
done

for fixture in "${iir_fixtures[@]}"; do
  start_ms="$(date +%s%3N)"
  chunk2_log_event "$log_file" "chunk2.pipeline.start" fixture "$fixture" profile infer_ir run_index 1
  golden_json="$f_b5_golden/$fixture/infer_ir.json"
  hashes_toml="$f_b5_golden/$fixture/hashes.toml"
  [[ -f "$golden_json" ]] || {
    chunk2_log_event "$log_file" "chunk2.pipeline.golden.diff" \
      fixture "$fixture" stage infer_ir expected "$golden_json" observed missing diff_path ""
    fail "$fixture" "missing F-B5 infer_ir golden"
  }
  [[ -f "$hashes_toml" ]] || fail "$fixture" "missing F-B5 infer_ir hash manifest"
  expected_iir_hash="$(tr -d '\n' < "fixtures/infer_ir/$fixture/infer_ir_self_hash")"
  observed_iir_hash="$(json_field "$golden_json" result.infer_ir_self_hash)"
  [[ "$observed_iir_hash" == "$expected_iir_hash" ]] || {
    chunk2_log_event "$log_file" "chunk2.pipeline.golden.diff" \
      fixture "$fixture" stage infer_ir expected "$expected_iir_hash" observed "$observed_iir_hash" diff_path "$golden_json"
    fail "$fixture" "infer_ir hash drift"
  }
  report_hash="$(toml_value "$hashes_toml" report_self_hash)"
  [[ -n "$report_hash" ]] || fail "$fixture" "missing report_self_hash in hashes.toml"
  golden_hash="$(chunk2_sha256_file "$golden_json")"
  chunk2_log_event "$log_file" "chunk2.pipeline.stage.complete" \
    fixture "$fixture" stage infer_ir report_self_hash "$report_hash"
  chunk2_log_event "$log_file" "chunk2.pipeline.golden.match" \
    fixture "$fixture" stage infer_ir golden_hash "$golden_hash"
  end_ms="$(date +%s%3N)"
  chunk2_log_event "$log_file" "chunk2.pipeline.complete" \
    fixture "$fixture" total_ms "$((end_ms - start_ms))" all_stages_passed true status verified
done

printf '%s\n' "$log_file"
