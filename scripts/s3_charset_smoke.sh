#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s3_charset_smoke.sh [--capture-events PATH]

Runs the F-S3 charset_v1 smoke check. B23 owns the eventual
`gbf s3 normalize-corpus` CLI entrypoint; until that command lands, this
script drives the same charset operation through the subscriber-level
logging test and validates the captured NDJSON event shape.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
capture_events="/tmp/s3-charset.ndjson"

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
    S3_CHARSET_CAPTURE_EVENTS="$capture_events" \
        cargo test -p gbf-experiments --features s3 \
            --test charset_logging_s3 \
            -- --exact charset_pipeline_logging_emits_started_complete_and_per_example_events
)

python3 - "$capture_events" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
events = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]

def by_name(name):
    return [event for event in events if event.get("fields", {}).get("event_name") == name]

started = by_name("s3::charset::pipeline_started")
examples = by_name("s3::charset::example_normalized")
complete = by_name("s3::charset::pipeline_complete")

assert len(started) == 1, f"expected one pipeline_started event, got {len(started)}"
assert len(complete) == 1, f"expected one pipeline_complete event, got {len(complete)}"
assert examples, "expected at least one example_normalized event"

for key in ("raw_train_byte_count", "raw_val_byte_count", "charset_v1_sha256"):
    assert key in started[0]["fields"], f"pipeline_started missing {key}"

for key in (
    "train_post_char_count",
    "val_post_char_count",
    "unmappable_example_drop_rate_train",
    "unmappable_example_drop_rate_val",
    "unmappable_char_drop_rate_train",
    "unmappable_char_drop_rate_val",
    "charset_self_hash",
):
    assert key in complete[0]["fields"], f"pipeline_complete missing {key}"

for event in examples:
    assert event["fields"].get("dropped") == "false", "smoke fixture unexpectedly dropped"

print(f"S3 charset smoke PASS events={path}")
PY
