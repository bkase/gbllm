#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="${TMPDIR:-/tmp}/s3_bundle_export_smoke.$$"
EVENTS_A="$TMP_ROOT/events-a.ndjson"
EVENTS_B="$TMP_ROOT/events-b.ndjson"
BUNDLE_A="$TMP_ROOT/s3-bundle-a.json"
BUNDLE_B="$TMP_ROOT/s3-bundle-b.json"
METADATA_A="$TMP_ROOT/s3-bundle-a.metadata.json"
METADATA_B="$TMP_ROOT/s3-bundle-b.metadata.json"

mkdir -p "$TMP_ROOT"
trap 'rm -rf "$TMP_ROOT"' EXIT

cd "$ROOT"

cargo run -q -p gbf-cli --features "s3,s3-phase-d" -- \
    --capture-events "$EVENTS_A" \
    s3 export-bundle \
    --seed 0 \
    --bundle-output "$BUNDLE_A" \
    --metadata-output "$METADATA_A" >/dev/null

cargo run -q -p gbf-cli --features "s3,s3-phase-d" -- \
    --capture-events "$EVENTS_B" \
    s3 export-bundle \
    --seed 0 \
    --bundle-output "$BUNDLE_B" \
    --metadata-output "$METADATA_B" >/dev/null

python3 - "$EVENTS_A" "$EVENTS_B" "$BUNDLE_A" "$BUNDLE_B" "$METADATA_A" "$METADATA_B" <<'PY'
import json
import sys
from pathlib import Path

events_a = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line]
events_b = [json.loads(line) for line in Path(sys.argv[2]).read_text().splitlines() if line]
bundle_a = Path(sys.argv[3]).read_bytes()
bundle_b = Path(sys.argv[4]).read_bytes()
metadata_a = json.loads(Path(sys.argv[5]).read_text())
metadata_b = json.loads(Path(sys.argv[6]).read_text())

required = {
    "s3::bundle_export::started",
    "s3::bundle_export::tensor_emitted",
    "s3::bundle_export::program_emitted",
    "s3::bundle_export::program_validated",
    "s3::bundle_export::complete",
}

observed = {event["fields"].get("event_name") for event in events_a}
missing = sorted(required - observed)
if missing:
    raise SystemExit(f"missing bundle export events: {missing}")

def complete_sha(events):
    complete = [
        event for event in events
        if event["fields"].get("event_name") == "s3::bundle_export::complete"
    ]
    if len(complete) != 1:
        raise SystemExit(f"expected one complete event, saw {len(complete)}")
    return complete[0]["fields"]["canonical_bundle_payload_sha"]

if complete_sha(events_a) != complete_sha(events_b):
    raise SystemExit("canonical_bundle_payload_sha changed across replay")
if bundle_a != bundle_b:
    raise SystemExit("canonical bundle bytes changed across replay")

if metadata_a != metadata_b:
    raise SystemExit("bundle metadata changed across replay")
if metadata_a.get("schema") != "s3_bundle.v1":
    raise SystemExit(f"metadata schema mismatch: {metadata_a.get('schema')!r}")

validation = metadata_a.get("program_validation", {})
if not validation.get("structural_valid"):
    raise SystemExit("program validation did not report structural_valid=true")
if not validation.get("argmax_token_all_match"):
    raise SystemExit("program validation did not report argmax_token_all_match=true")
if validation.get("semantic_max_logit_abs_diff", 1.0) > 0.000004:
    raise SystemExit("program validation exceeded Phase-A logit tolerance")
PY
