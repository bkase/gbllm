#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

out_dir="${1:-docs/review/f-b3/golden}"
log_dir="${GBF_REVIEW_LOG_DIR:-target/e2e-logs}"
log_file="$log_dir/f-b3-review-regen.jsonl"
mkdir -p "$log_dir"
: > "$log_file"

if [[ -f scripts/e2e/lib/chunk2_logging.sh ]]; then
  # shellcheck disable=SC1091
  source scripts/e2e/lib/chunk2_logging.sh
fi

log_event() {
  local event="$1"
  local status="$2"
  local fixture="${3:-}"
  local detail="${4:-}"
  printf '{"event":"%s","status":"%s","fixture":"%s","detail":"%s"}\n' \
    "$event" "$status" "$fixture" "$detail" >> "$log_file"
}

read_toml_string() {
  local key="$1"
  local path="$2"
  sed -n "s/^${key} = \"\\(.*\\)\"/\\1/p" "$path"
}

passing_fixtures=(
  dense_toy0
  dense_toy1_tied
  dense_toy1_untied
  routed_basic_one
  routed_basic_selected_score
  mixed_topology
)

rm -rf "$out_dir"
mkdir -p "$out_dir/pass" "$out_dir/reject"

log_event "f_b3_review.regen" "start" "" "$out_dir"
cargo test -q -p gbf-codegen --lib s1::quant_graph::tests::fixture_quant_graph -- --test-threads=1
log_event "f_b3_review.fixture_test" "ok" "fixture_quant_graph" "cargo"

printf 'fixture\tdescription\tbuilder\tquant_graph_self_hash\tquant_graph_canonical_bytes_hash\treport_self_hash\n' \
  > "$out_dir/pass/passing-fixtures.tsv"

for fixture in "${passing_fixtures[@]}"; do
  src="fixtures/quant_graph/$fixture"
  dst="$out_dir/pass/$fixture"
  mkdir -p "$dst"
  cp "$src/fixture.toml" "$dst/fixture.toml"
  cp "$src/quant_graph_self_hash" "$dst/quant_graph_self_hash"
  cp "$src/quant_graph_canonical_bytes_hash" "$dst/quant_graph_canonical_bytes_hash"
  cp "$src/report_self_hash" "$dst/report_self_hash"

  description="$(read_toml_string description "$src/fixture.toml")"
  builder="$(read_toml_string builder "$src/fixture.toml")"
  qg_hash="$(tr -d '\n' < "$src/quant_graph_self_hash")"
  qg_canonical_hash="$(tr -d '\n' < "$src/quant_graph_canonical_bytes_hash")"
  report_hash="$(tr -d '\n' < "$src/report_self_hash")"

  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$fixture" "$description" "$builder" "$qg_hash" "$qg_canonical_hash" "$report_hash" \
    >> "$out_dir/pass/passing-fixtures.tsv"

  cat > "$dst/quant_graph.json" <<EOF
{
  "schema": "f-b3.review.quant_graph_golden.v1",
  "fixture": "$fixture",
  "description": "$description",
  "builder": "$builder",
  "quant_graph_self_hash": "$qg_hash",
  "quant_graph_canonical_bytes_hash": "$qg_canonical_hash",
  "report_self_hash": "$report_hash"
}
EOF
  log_event "f_b3_review.pass_golden" "ok" "$fixture" "$qg_hash"
done

printf 'qg_reject\tfixture\tdiagnostic_code\tseverity\tclause\tcounterexample\n' \
  > "$out_dir/reject/reject-classes.tsv"
cp fixtures/quant_graph/reject/README.md "$out_dir/reject/README.md"

reject_count=0
while IFS= read -r expected; do
  src_dir="$(dirname "$expected")"
  fixture="$(basename "$src_dir")"
  dst_dir="$out_dir/reject/$fixture"
  mkdir -p "$dst_dir"
  cp "$src_dir/expected.toml" "$dst_dir/expected.toml"
  cp "$src_dir/inputs.toml" "$dst_dir/inputs.toml"
  cp "$src_dir/README.md" "$dst_dir/README.md"

  qg_reject="$(sed -n 's/^qg_reject = //p' "$src_dir/expected.toml")"
  diagnostic_code="$(read_toml_string diagnostic_code "$src_dir/expected.toml")"
  severity="$(read_toml_string severity "$src_dir/expected.toml")"
  clause="$(read_toml_string clause "$src_dir/expected.toml")"
  counterexample="$(read_toml_string counterexample "$src_dir/inputs.toml")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$qg_reject" "$fixture" "$diagnostic_code" "$severity" "$clause" "$counterexample" \
    >> "$out_dir/reject/reject-classes.tsv"
  reject_count=$((reject_count + 1))
  log_event "f_b3_review.reject_golden" "ok" "$fixture" "$diagnostic_code"
done < <(find fixtures/quant_graph/reject -mindepth 2 -maxdepth 2 -name expected.toml | sort)

if [[ "$reject_count" -ne 36 ]]; then
  log_event "f_b3_review.reject_count" "failed" "" "$reject_count"
  printf 'expected 36 reject fixtures, found %s\n' "$reject_count" >&2
  exit 1
fi

cat > "$out_dir/manifest.json" <<EOF
{
  "schema": "f-b3.review.golden_manifest.v1",
  "passing_fixture_count": ${#passing_fixtures[@]},
  "reject_fixture_count": $reject_count,
  "source": "fixtures/quant_graph",
  "pipeline_gate": "cargo test -q -p gbf-codegen --lib s1::quant_graph::tests::fixture_quant_graph -- --test-threads=1"
}
EOF

log_event "f_b3_review.regen" "ok" "" "$out_dir"
