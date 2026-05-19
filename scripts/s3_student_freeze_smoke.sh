#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s3_student_freeze_smoke.sh [--capture-events PATH]

Runs the F-S3 student-freeze smoke check. B14 owns full artifact export; this
script drives B12 through the subscriber-level logging test and validates that
both the train-side student_freeze event and the s3_phase_log.v1 JSONL producer
event are emitted with matching fingerprints.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
capture_events="/tmp/s3-student-freeze.ndjson"

while (($#)); do
    case "$1" in
        --capture-events)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --capture-events requires a value" >&2
                exit 2
            fi
            capture_events="$1"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

mkdir -p "$(dirname "$capture_events")"
rm -f "$capture_events"

(
    cd "$repo_root"
    S3_STUDENT_FREEZE_CAPTURE_EVENTS="$capture_events" \
        cargo test -p gbf-experiments --features s3 \
            --test student_freeze_logging_s3 \
            -- --exact student_freeze_logging_emits_train_and_phase_log_events_once
)

python3 - "$capture_events" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
events = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]

def by_name(name):
    return [event for event in events if event.get("fields", {}).get("event_name") == name]

student = by_name("s3::student_freeze")
phase_log = by_name("s3_phase_log.v1")

assert len(student) == 1, f"expected one s3::student_freeze event, got {len(student)}"
assert len(phase_log) == 1, f"expected one s3_phase_log.v1 event, got {len(phase_log)}"

student_fields = student[0]["fields"]
phase_fields = phase_log[0]["fields"]

assert student[0]["target"] == "gbf_train::student", student[0]["target"]
assert phase_log[0]["target"] == "gbf_experiments::s3", phase_log[0]["target"]
assert student_fields.get("step") == "10001", student_fields.get("step")
assert phase_fields.get("step") == "10001", phase_fields.get("step")
assert phase_fields.get("schema") == "s3_phase_log.v1", phase_fields.get("schema")
assert phase_fields.get("event_kind") == "student_freeze", phase_fields.get("event_kind")

for key in (
    "student_storage_fingerprint",
    "student_weight_fingerprint",
):
    assert student_fields.get(key), f"student event missing {key}"
    assert phase_fields.get(key) == student_fields.get(key), f"phase-log {key} mismatch"

for key in ("source_storage_identity", "frozen_storage_identity"):
    assert student_fields.get(key), f"student event missing {key}"

print(f"S3 student freeze smoke PASS events={path}")
PY
