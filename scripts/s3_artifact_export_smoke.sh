#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-artifact-export.XXXXXX")"
EVENTS="$RUN_DIR/s3-artifact.ndjson"
ARTIFACT1="$RUN_DIR/seed-0/artifact-1.bin"
METADATA1="$RUN_DIR/seed-0/artifact-metadata.json"
ARTIFACT2="$RUN_DIR/seed-0/replay/artifact-2.bin"
METADATA2="$RUN_DIR/seed-0/replay/artifact-metadata.json"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

# Fixture-only smoke: `gbf s3 export-artifact` freezes and exports the
# deterministic in-repo FixtureArtifactStudent. It does not run Phase-D
# training; B18 owns the real runner/e2e training integration.
cargo run -q -p gbf-cli --features s3-full -- \
  --capture-events "$EVENTS" \
  s3 export-artifact \
  --seed 0 \
  --artifact-output "$ARTIFACT1" \
  --metadata-output "$METADATA1" >/dev/null

cargo run -q -p gbf-cli --features s3-full -- \
  s3 export-artifact \
  --seed 0 \
  --artifact-output "$ARTIFACT2" \
  --metadata-output "$METADATA2" >/dev/null

python3 - "$METADATA1" "$METADATA2" "$EVENTS" "$ARTIFACT1" <<'PY'
import json
import sys
from pathlib import Path

first = json.loads(Path(sys.argv[1]).read_text())
second = json.loads(Path(sys.argv[2]).read_text())
events = [
    json.loads(line)
    for line in Path(sys.argv[3]).read_text().splitlines()
    if line.strip()
]
artifact = json.loads(Path(sys.argv[4]).read_text())

assert first["schema"] == "s3_artifact.v1", first
assert artifact["core"]["model"]["model_id"].startswith("fixture-artifact-student-"), artifact[
    "core"
]["model"]
assert first["canonical_artifact_payload_sha"] == second["canonical_artifact_payload_sha"], (
    first["canonical_artifact_payload_sha"],
    second["canonical_artifact_payload_sha"],
)
assert first["weight_resolution_summary"]["tensors_resolved_via_naming"] == 0, first
assert first["weight_resolution_summary"]["tensors_resolved_via_quant_spec"] == first[
    "weight_resolution_summary"
]["total_tensors"], first

s3_events = [
    event
    for event in events
    if event.get("target") == "gbf_experiments::s3::artifact"
]
names = [event["fields"].get("event_name") for event in s3_events]
for name in [
    "s3::artifact_export::started",
    "s3::artifact_export::tensor_emitted",
    "s3::artifact_export::quantspec_validated",
    "s3::artifact_export::tied_alias_recorded",
    "s3::artifact_export::complete",
]:
    assert name in names, names
PY

"$ROOT/scripts/s3_no_naming_resolution_check.sh" "$RUN_DIR"
