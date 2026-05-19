#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="${TMPDIR:-/tmp}/s3_oracle_agreement_smoke.$$"
PRODUCT="$TMP_ROOT/agreement-product.json"
EVENTS="$TMP_ROOT/agreement-events.ndjson"

mkdir -p "$TMP_ROOT"
trap 'rm -rf "$TMP_ROOT"' EXIT

cd "$ROOT"

S3_ORACLE_AGREEMENT_PRODUCT_OUT="$PRODUCT" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test oracle_agreement_s3 oracle_agreement_s3

S3_ORACLE_AGREEMENT_CAPTURE_EVENTS="$EVENTS" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test oracle_agreement_logging_s3 oracle_agreement_logging_s3

python3 - "$PRODUCT" "$EVENTS" <<'PY'
import json
import sys
from pathlib import Path

expected_hash = "sha256:50945182ee8769112f664e680133f30978ea1b3f312f1b41abe03bd1b6d1d69b"
expected_records = 5 * 2 * 3 * 16 * 2

product = json.loads(Path(sys.argv[1]).read_text())
assert product["schema"] == "s3_oracle_agreement.v1", product["schema"]
assert len(product["records"]) == expected_records, len(product["records"])
assert product["overall_pass"] is True, product
assert product["phase_a_pass"] is True, product
assert product["phase_d_pass"] is True, product
assert product["live_observation_source"] == {
    "kind": "oracle_derived_fixture",
    "real_owner_bead": "bd-1ybu",
}, product["live_observation_source"]
assert product["fallback_used"] == ["S3LiveObservationFixture"], product["fallback_used"]
assert product["agreement_self_hash"] == expected_hash, product["agreement_self_hash"]

events = [
    json.loads(line)
    for line in Path(sys.argv[2]).read_text().splitlines()
    if line
]
agreement_events = [
    event for event in events
    if event.get("target") == "gbf_experiments::s3::oracle"
]
names = [event["fields"].get("event_name") for event in agreement_events]
assert names.count("s3::agreement::run_started") == 1, names
assert names.count("s3::agreement::run_complete") == 1, names
assert names.count("s3::agreement::live_observation_captured") == expected_records, names.count("s3::agreement::live_observation_captured")
assert names.count("s3::agreement::record_emitted") == expected_records, names.count("s3::agreement::record_emitted")

started = next(
    event for event in agreement_events
    if event["fields"].get("event_name") == "s3::agreement::run_started"
)
assert started["fields"]["seed_count"] == "5", started
assert started["fields"]["prompt_subset_size"] == "3", started
assert started["fields"]["agreement_trace_steps"] == "16", started
assert started["fields"]["stop_on_eos"] == "false", started
assert started["fields"]["live_observation_source"] == "oracle_derived_fixture", started
assert started["fields"]["live_observation_real_owner_bead"] == "bd-1ybu", started
assert started["fields"]["live_observation_count"] == str(expected_records), started

complete = next(
    event for event in agreement_events
    if event["fields"].get("event_name") == "s3::agreement::run_complete"
)
assert complete["fields"]["total_records"] == str(expected_records), complete
assert complete["fields"]["overall_pass"] == "true", complete
assert complete["fields"]["live_observation_source"] == "oracle_derived_fixture", complete
assert complete["fields"]["live_observation_real_owner_bead"] == "bd-1ybu", complete
assert "S3LiveObservationFixture" in complete["fields"]["fallback_used"], complete
assert complete["fields"]["agreement_self_hash"] == expected_hash, complete
PY
