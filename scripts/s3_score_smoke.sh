#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

events="${S3_SCORE_CAPTURE_NDJSON:-/tmp/s3-score.ndjson}"
rm -f "$events"

S3_SCORE_CAPTURE_NDJSON="$events" \
  cargo test -q -p gbf-experiments --features s3 --test score_smoke_s3 \
  score_smoke_fixture_reference_beats_kn_and_writes_optional_ndjson

require_line() {
  local needle="$1"
  local path="$2"
  if command -v rg >/dev/null 2>&1; then
    rg --fixed-strings --quiet "$needle" "$path"
  else
    grep -Fq "$needle" "$path"
  fi
}

require_line '"name":"s3::score::started"' "$events"
require_line '"name":"s3::score::chunk_complete"' "$events"
require_line '"name":"s3::score::complete"' "$events"

printf 'verified %s\n' "$events"
