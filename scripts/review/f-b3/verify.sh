#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

golden_dir="docs/review/f-b3/golden"
log_dir="${GBF_REVIEW_LOG_DIR:-target/e2e-logs}"
log_file="$log_dir/f-b3-review-verify.jsonl"
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

fail() {
  log_event "f_b3_review.verify" "failed" "${1:-}" "${2:-}"
  printf 'f-b3 verify: %s %s\n' "${1:-}" "${2:-}" >&2
  exit 1
}

log_event "f_b3_review.verify" "start" "" "$golden_dir"

if grep -REn '\b([c]url|[w]get|[n]px|[b]unx|[g]h[[:space:]]|[g]it[[:space:]]+(fetch|pull|clone|push))\b' \
  scripts/review/f-b3 >/tmp/f-b3-review-network-grep.$$; then
  cat /tmp/f-b3-review-network-grep.$$
  rm -f /tmp/f-b3-review-network-grep.$$
  fail "network" "network-capable command found"
fi
rm -f /tmp/f-b3-review-network-grep.$$

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
scripts/review/f-b3/regen.sh "$tmp/golden"
diff -ur "$golden_dir" "$tmp/golden"
log_event "f_b3_review.golden_diff" "ok" "" "$golden_dir"

row_count="$(grep -c '^| QG-Reject-' docs/review/f-b3/reject-class-table.md)"
[[ "$row_count" -eq 36 ]] || fail "reject-class-table.md" "expected 36 rows got $row_count"

while IFS=$'\t' read -r qg_reject fixture diagnostic_code severity clause counterexample; do
  if [[ "$qg_reject" == "qg_reject" ]]; then
    continue
  fi
  fixture_path="fixtures/quant_graph/reject/$fixture"
  grep -Fq "QG-Reject-$qg_reject" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing QG-Reject-$qg_reject"
  grep -Fq "$fixture_path" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing fixture path"
  grep -Fq "$diagnostic_code" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing diagnostic code"
  grep -Fq "$severity" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing severity"
  grep -Fq "$counterexample" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing counterexample"
  grep -Fq "$clause" docs/review/f-b3/reject-class-table.md \
    || fail "$fixture" "missing clause"
done < "$golden_dir/reject/reject-classes.tsv"

for fixture in dense_toy0 dense_toy1_tied dense_toy1_untied routed_basic_one routed_basic_selected_score mixed_topology; do
  test -f "$golden_dir/pass/$fixture/quant_graph.json" \
    || fail "$fixture" "missing quant_graph.json golden"
  grep -Fq "$fixture" docs/review/f-b3/SUMMARY.md \
    || fail "$fixture" "missing SUMMARY row"
done

grep -Fq "router.<layer>" docs/review/f-b3/reduction-site-id-scheme.md \
  || fail "reduction-site-id-scheme.md" "missing router scheme"
grep -Fq "expert.<layer>.<expert>.<slot>" docs/review/f-b3/reduction-site-id-scheme.md \
  || fail "reduction-site-id-scheme.md" "missing expert scheme"
grep -Fq "norm.<norm_plan_id>" docs/review/f-b3/reduction-site-id-scheme.md \
  || fail "reduction-site-id-scheme.md" "missing norm scheme"
grep -Fq "classify" docs/review/f-b3/reduction-site-id-scheme.md \
  || fail "reduction-site-id-scheme.md" "missing classify scheme"

for persona in P1 P2 P4 P5 P6 P8; do
  grep -Fq "$persona" docs/review/f-b3/SUMMARY.md \
    || fail "SUMMARY.md" "missing persona $persona"
done

log_event "f_b3_review.verify" "ok" "" "$golden_dir"
