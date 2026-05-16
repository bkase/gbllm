#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-conformance.XXXXXX")"
FIRST="$RUN_DIR/first/experiments/S3/conformance/conformance.json"
SECOND="$RUN_DIR/second/experiments/S3/conformance/conformance.json"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

S3_CONFORMANCE_OUT="$FIRST" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test conformance_round_trip_s3 conformance_round_trip_s3

S3_CONFORMANCE_OUT="$SECOND" \
  cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" \
    --test conformance_round_trip_s3 conformance_round_trip_s3

python3 - "$FIRST" "$SECOND" <<'PY'
import json
import sys
from pathlib import Path

expected_hash = "sha256:1f200ff41cbfffc35ebe5b058b9fd22ce2fc60fc644371acacf1961a922ea0dc"

first_path = Path(sys.argv[1])
second_path = Path(sys.argv[2])
first_bytes = first_path.read_bytes()
second_bytes = second_path.read_bytes()
assert first_bytes == second_bytes, "canonical conformance bytes changed across replay"

product = json.loads(first_bytes)
assert product["schema"] == "s3_conformance.v1", product
assert product["real_owner_bead"] == "bd-35l3", product
assert product["conformance_self_hash"] == expected_hash, product["conformance_self_hash"]
assert len(product["per_seed"]) == 5, product["per_seed"]
assert product["overall"]["passed"] is True, product["overall"]
for seed in product["per_seed"]:
    assert "post_logits" in seed["per_checkpoint"], seed
    assert "post_decode" in seed["per_checkpoint"], seed
    assert any("max_abs_logit_diff" in key for key in seed["per_metric"]), seed
PY
