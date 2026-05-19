#!/usr/bin/env bash
set -euo pipefail

# B18 smoke intentionally drives the in-repo v0_success runner through focused
# cargo tests with env-captured product/events. The public
# `gbf s3 replay-full` CLI surface is a B23 handoff; when B23 lands, replace
# this harness entrypoint with the CLI replay while preserving the JSON/event
# assertions below.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-v0-success.XXXXXX")"
PRODUCT="$RUN_DIR/v0-success-product.json"
EVENTS="$RUN_DIR/v0-success-events.ndjson"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_per_prompt_aggregation_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_terminal_eos_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_outcome_totality_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_chrome_budget_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_canonical_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test contamination_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
  --test v0_success_proptest_s3

S3_V0_SUCCESS_PRODUCT_OUT="$PRODUCT" \
  S3_V0_SUCCESS_CAPTURE_EVENTS="$EVENTS" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test v0_success_logging_s3

python3 - "$PRODUCT" "$EVENTS" <<'PY'
import json
import sys
from pathlib import Path

expected_hash = "sha256:83215ba387b509313183bd97d580c18b56bd3ef85f4347e9caa9b35367d125b8"
expected_seed_count = 5
expected_prompt_count = 8

product = json.loads(Path(sys.argv[1]).read_text())
assert product["schema"] == "s3_v0_success.v1", product
assert product["overall_pass"] is True, product
assert product["suspicious_low_bpc"] is False, product
assert product["v0_success_self_hash"] == expected_hash, product["v0_success_self_hash"]
assert len(product["per_seed"]) == expected_seed_count, product["per_seed"]
for seed in product["per_seed"]:
    assert seed["pass"] is True, seed
    for gate in ["Q1_holds", "Q2_holds", "Q3_holds", "Q4_holds", "Q5_holds", "Q6_holds"]:
        assert seed[gate] is True, seed
    assert len(seed["per_prompt_generation"]) == expected_prompt_count, seed

events = [
    json.loads(line)
    for line in Path(sys.argv[2]).read_text().splitlines()
    if line.strip()
]
workload_events = [
    event
    for event in events
    if event.get("target") == "gbf_experiments::s3::workload"
]
names = [event["fields"].get("event_name") for event in workload_events]
assert names.count("s3::v0_success::run_started") == 1, names
assert names.count("s3::v0_success::seed_started") == expected_seed_count, names
assert names.count("s3::v0_success::generation_per_prompt") == expected_seed_count * expected_prompt_count, names.count("s3::v0_success::generation_per_prompt")
assert names.count("s3::v0_success::scoring_complete") == expected_seed_count, names
assert names.count("s3::v0_success::quality_gate") == expected_seed_count, names
assert names.count("s3::v0_success::run_complete") == 1, names
assert names.count("s3::contamination::checked") == 1, names

complete = next(
    event for event in workload_events
    if event["fields"].get("event_name") == "s3::v0_success::run_complete"
)
assert complete["fields"]["overall_pass"] is True, complete
assert complete["fields"]["suspicious_low_bpc"] is False, complete
assert complete["fields"]["v0_success_self_hash"] == expected_hash, complete
PY

"$ROOT/scripts/s3_artifact_export_smoke.sh"
