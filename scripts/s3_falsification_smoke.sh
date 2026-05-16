#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-falsification.XXXXXX")"
EVENTS="$RUN_DIR/s3-falsify.ndjson"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

cargo test -p gbf-experiments \
  --features "s3,s3-phase-d,s3-oracle-real,s3-oracle-adversarial,falsify" \
  --test falsification_s3 -- --nocapture

S3_FALSIFICATION_CAPTURE_EVENTS="$EVENTS" \
  cargo test -p gbf-experiments --features falsify \
    --test falsification_s3_logging_s3 -- --nocapture

cargo test -p gbf-experiments --features falsify \
  --test falsification_s3_suite_hash -- --nocapture

python3 - "$EVENTS" <<'PY'
import json
import sys
from pathlib import Path

expected_hash = "sha256:916700709ed532667de7788d1b8373baafe0fe715d1652ca787789a9e4e0a248"
events_path = Path(sys.argv[1])
events = [
    json.loads(line)
    for line in events_path.read_text().splitlines()
    if line.strip()
]
complete = [
    event
    for event in events
    if event["name"] == "s3::falsify::substitute_complete"
]
assert len(complete) == 9, complete
assert all(event["fields"]["matches_expected"] is True for event in complete), complete
started = [
    event
    for event in events
    if event["name"] == "s3::falsify::suite_started"
]
assert len(started) == 1, started
assert started[0]["fields"]["suite_hash"] == expected_hash, started
assert any(
    event["name"] == "s3_phase_log.v1"
    and event["fields"].get("event_kind") == "student_freeze"
    for event in events
), events
PY
