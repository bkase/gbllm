#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-report.XXXXXX")"
FIRST="$RUN_DIR/S3-report.first.md"
SECOND="$RUN_DIR/S3-report.second.md"
EVENTS="$RUN_DIR/s3-report.ndjson"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

S3_REPORT_OUT="$FIRST" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test report_emitter_canonical_s3 -- --nocapture

S3_REPORT_CAPTURE_EVENTS="$EVENTS" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test report_logging_s3 -- --nocapture

S3_REPORT_OUT="$SECOND" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test report_emitter_canonical_s3 -- --nocapture

python3 - "$FIRST" "$SECOND" "$EVENTS" <<'PY'
import json
import sys
from pathlib import Path

first = Path(sys.argv[1]).read_bytes()
second = Path(sys.argv[2]).read_bytes()
assert first == second, "s3_report.v1 markdown changed across replay"
assert b'"schema":"s3_report.v1"' in first, first[:200]
assert b"## Pre-registered predictions" in first, first[:200]

events = [
    json.loads(line)
    for line in Path(sys.argv[3]).read_text().splitlines()
    if line.strip()
]
names = [event["name"] for event in events]
assert "s3::report::emission_started" in names, names
assert "s3::report::emission_complete" in names, names
validators = [
    event for event in events
    if event["name"] == "s3::report::r_validator_passed"
]
assert len(validators) == 7, validators
PY
