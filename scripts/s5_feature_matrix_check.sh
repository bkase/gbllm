#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN=0
SAMPLE=0
REPORT_PATH="/tmp/s5-feature-matrix.json"

usage() {
  cat <<'USAGE'
usage: scripts/s5_feature_matrix_check.sh [--dry-run] [--sample] [--report-path PATH]

Checks the legal F-S5 closure feature combinations without using --all-features.
Default mode runs the full closure matrix:
  - s5-default,qat,burn-adapter
  - s5-no-log,qat,burn-adapter
  - s5-default,qat,burn-adapter,s5-falsify-N for N=1..15

--sample limits falsifier rows to N=1,14,15 for local smoke runs.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --sample)
      SAMPLE=1
      shift
      ;;
    --report-path)
      REPORT_PATH="${2:?--report-path requires a path}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "$ROOT"

json_escape() {
  python3 -c 'import json, sys; print(json.dumps(sys.stdin.read()))'
}

emit_event() {
  local event="$1"
  local name="$2"
  local features="$3"
  local mode="$4"
  local status="$5"
  local detail="${6:-}"
  local escaped_detail
  escaped_detail="$(printf '%s' "$detail" | json_escape)"
  printf '{"event":"%s","name":"%s","features":"%s","mode":"%s","status":"%s","detail":%s}\n' \
    "$event" "$name" "$features" "$mode" "$status" "$escaped_detail" >&2
}

run_cmd() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'DRY-RUN %q' "$@"
    printf '\n'
    return 0
  fi
  "$@"
}

run_expected_failure() {
  local name="$1"
  local features="$2"
  local expected="$3"
  local output
  local status

  emit_event "s5_feature_matrix_stage_start" "$name" "$features" "expect-fail" "running"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    emit_event "s5_feature_matrix_stage_done" "$name" "$features" "expect-fail" "dry-run" "$expected"
    return 0
  fi

  set +e
  output=$(cargo check -p gbf-experiments --no-default-features --features "$features" 2>&1)
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "expected feature mutex row $name to fail, but it succeeded" >&2
    exit 1
  fi
  if ! grep -Fq "$expected" <<<"$output"; then
    echo "feature mutex row $name failed without expected diagnostic: $expected" >&2
    echo "$output" >&2
    exit 1
  fi
  emit_event "s5_feature_matrix_stage_done" "$name" "$features" "expect-fail" "passed" "$expected"
}

rows=(
  "s5-default|s5-default,qat,burn-adapter|test-no-run"
  "s5-no-log|s5-no-log,qat,burn-adapter|test-no-run"
)

if [[ "$SAMPLE" -eq 1 ]]; then
  falsifiers=(1 14 15)
else
  falsifiers=(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
fi

for n in "${falsifiers[@]}"; do
  rows+=("s5-falsify-${n}|s5-default,qat,burn-adapter,s5-falsify-${n}|check")
done

mkdir -p "$(dirname "$REPORT_PATH")"
: > "$REPORT_PATH"
printf '{"script":"s5_feature_matrix_check","dry_run":%s,"sample":%s,"rows":[\n' \
  "$([[ "$DRY_RUN" -eq 1 ]] && echo true || echo false)" \
  "$([[ "$SAMPLE" -eq 1 ]] && echo true || echo false)" >> "$REPORT_PATH"

first=1
for row in "${rows[@]}"; do
  IFS='|' read -r name features mode <<<"$row"
  emit_event "s5_feature_matrix_stage_start" "$name" "$features" "$mode" "running"
  if [[ "$mode" == "test-no-run" ]]; then
    run_cmd cargo test -p gbf-experiments --lib --no-run --no-default-features --features "$features"
  else
    run_cmd cargo check -p gbf-experiments --no-default-features --features "$features"
  fi
  emit_event "s5_feature_matrix_stage_done" "$name" "$features" "$mode" "passed"

  if [[ "$first" -eq 0 ]]; then
    printf ',\n' >> "$REPORT_PATH"
  fi
  first=0
  printf '  {"name":"%s","features":"%s","mode":"%s"}' "$name" "$features" "$mode" >> "$REPORT_PATH"
done

printf '\n],"mutex_checks":[' >> "$REPORT_PATH"
run_expected_failure \
  "s5-default-vs-s5-no-log" \
  "s5-default,s5-no-log,qat,burn-adapter" \
  "S5 feature mutex violated: s5-default and s5-no-log are mutually exclusive"
printf '{"name":"s5-default-vs-s5-no-log","features":"s5-default,s5-no-log,qat,burn-adapter"}' >> "$REPORT_PATH"
printf ',' >> "$REPORT_PATH"
run_expected_failure \
  "s5-falsify-pair" \
  "s5-falsify-14,s5-falsify-15,qat,burn-adapter" \
  "S5 falsifier feature mutex violated: enable at most one s5-falsify-N feature"
printf '{"name":"s5-falsify-pair","features":"s5-falsify-14,s5-falsify-15,qat,burn-adapter"}' >> "$REPORT_PATH"
printf '],"passed":true}\n' >> "$REPORT_PATH"

printf 'S5 feature matrix PASS dry_run=%s sample=%s rows=%s report=%s\n' \
  "$DRY_RUN" "$SAMPLE" "${#rows[@]}" "$REPORT_PATH"
