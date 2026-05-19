#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="${TMPDIR:-/tmp}/s3_denotational_smoke.$$"
REAL_BYTES="$TMP_ROOT/real-observations.json"
FALLBACK_BYTES="$TMP_ROOT/fallback-observations.json"
EVENTS="$TMP_ROOT/events.ndjson"

mkdir -p "$TMP_ROOT"
trap 'rm -rf "$TMP_ROOT"' EXIT

cd "$ROOT"

S3_DENOTATIONAL_OBSERVATIONS_OUT="$REAL_BYTES" \
  cargo test -p gbf-oracle --features s3-real \
    --test denotational_real_s3 denotational_real_evaluates_reference_bundle

S3_DENOTATIONAL_OBSERVATIONS_OUT="$FALLBACK_BYTES" \
  cargo test -p gbf-oracle --features s3-fallback \
    --test denotational_fallback_s3 denotational_fallback_evaluates_reference_bundle_and_records_owner

diff -u "$REAL_BYTES" "$FALLBACK_BYTES"

S3_DENOTATIONAL_CAPTURE_EVENTS="$EVENTS" \
  cargo test -p gbf-oracle --features s3-real \
    --test denotational_logging_s3 denotational_oracle_logging_emits_required_events

python3 - "$EVENTS" <<'PY'
import json
import sys
from pathlib import Path

events = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line]
target_events = [
    event for event in events
    if event.get("target") == "gbf_oracle::denotational"
]
names = [event["fields"].get("event_name") for event in target_events]
assert names.count("s3::denotational_oracle::evaluation_started") == 1, names
assert names.count("s3::denotational_oracle::evaluation_complete") == 1, names
assert names.count("s3::denotational_oracle::observation_captured") > 0, names

started = next(
    event for event in target_events
    if event["fields"].get("event_name") == "s3::denotational_oracle::evaluation_started"
)
assert started["fields"]["backend_kind"] == "real", started
assert started["fields"]["prompt_count"] == "3", started

complete = next(
    event for event in target_events
    if event["fields"].get("event_name") == "s3::denotational_oracle::evaluation_complete"
)
assert complete["fields"]["determinism_class"] == "BitExact", complete
assert complete["fields"]["observation_count"] == str(
    names.count("s3::denotational_oracle::observation_captured")
), complete
PY
