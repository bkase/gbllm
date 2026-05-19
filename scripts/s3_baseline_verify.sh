#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Default to the B10-owned deterministic oracle fixture. The TinyStories.v2
# manifest in this checkout is fixture-mode data for hash plumbing while the
# real external corpus remains outside the repo.
manifest="${S3_BASELINE_VERIFY_MANIFEST:-fixtures/baselines/kn_oracle/manifest.toml}"
events="${S3_BASELINE_VERIFY_NDJSON:-/tmp/s3-baseline.ndjson}"
output="${S3_BASELINE_VERIFY_JSON:-/tmp/s3-baseline-kn5.json}"
expected_self_hash="sha256:bfae1b37193cf55cb17ee88461311facb61c14427f9a03488bdc5cd89f114334"

cargo run -q -p gbf-cli --features s3 -- \
  --capture-events "$events" \
  s3 fit-baseline \
  --manifest "$manifest" \
  --output "$output" >/dev/null

require_line() {
  local needle="$1"
  local path="$2"
  if command -v rg >/dev/null 2>&1; then
    rg --fixed-strings --quiet "$needle" "$path"
  else
    grep -Fq "$needle" "$path"
  fi
}

require_line '"event_name":"s3::baseline::fit_started"' "$events"
require_line '"event_name":"s3::baseline::counts_computed"' "$events"
require_line '"event_name":"s3::baseline::discounts_fit"' "$events"
require_line '"d_1_order_2":' "$events"
require_line '"y_order_5":' "$events"
require_line '"event_name":"s3::baseline::scoring_complete"' "$events"
require_line '"schema":"s3_baseline_kn5.v1"' "$output"
require_line "\"baseline_self_hash\":\"${expected_self_hash}\"" "$output"

printf 'verified %s\n' "$output"
