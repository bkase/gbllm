#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-verify}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
cert_path="${1:-$out_dir/reports/stage5/chunked_i16/certs/range.cert.json}"
ndjson="$out_dir/verify-packet.ndjson"

if [[ ! -f "$cert_path" ]]; then
  echo "error: missing range certificate $cert_path" >&2
  exit 1
fi

mkdir -p "$out_dir"

cargo run -q -p gbf-verify --bin gbf-verify -- \
  range-cert verify "$cert_path" \
  --ndjson "$ndjson" \
  --build-id "$build_id"

cargo run -q -p gbf-verify --bin f_b6_f_b7_tamper_fixtures -- \
  "$cert_path" \
  "$out_dir"

tampered_cases=(
  "malformed_json:range_cert.independent_verify.failed.malformed"
  "report_self_hash_mismatch:range_cert.independent_verify.failed.report_self_hash_mismatch"
  "unsupported_plan_family:range_cert.independent_verify.failed.unsupported_plan_family"
  "cert_lowered_slack:range_cert.independent_verify.certified_reduction.chunked_i16"
  "cert_wrong_plan_family:range_cert.independent_verify.certified_reduction.single_i16"
  "cert_inconsistent_term_count:range_cert.independent_verify.certified_reduction.chunked_i16"
  "cert_failed_witness_mismatch:range_cert.independent_verify.failed.witness_mismatch"
)

for case_spec in "${tampered_cases[@]}"; do
  case_name="${case_spec%%:*}"
  tampered_cert="$out_dir/tampered/$case_name/range.cert.json"
  if [[ ! -f "$tampered_cert" ]]; then
    echo "error: missing generated tampered cert $tampered_cert" >&2
    exit 1
  fi
  set +e
  cargo run -q -p gbf-verify --bin gbf-verify -- \
    range-cert verify "$tampered_cert" \
    --ndjson "$ndjson" \
    --build-id "$build_id" >/tmp/f-b6-f-b7-cert-verify-stdout.txt 2>/tmp/f-b6-f-b7-cert-verify-stderr.txt
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "error: tampered cert unexpectedly verified: $tampered_cert" >&2
    cat /tmp/f-b6-f-b7-cert-verify-stdout.txt >&2
    cat /tmp/f-b6-f-b7-cert-verify-stderr.txt >&2
    exit 1
  fi
done

python3 - "$ndjson" "$out_dir" "$cert_path" "${tampered_cases[@]}" <<'PY'
import json
import pathlib
import sys

ndjson = pathlib.Path(sys.argv[1])
out_dir = pathlib.Path(sys.argv[2])
passing_cert = pathlib.Path(sys.argv[3])
case_specs = sys.argv[4:]
payloads = [json.loads(line) for line in ndjson.read_text(encoding="utf-8").splitlines()]

def has_event(cert_path, event, outcome):
    cert_path = str(cert_path)
    return any(
        payload.get("event") == event
        and payload.get("fields", {}).get("cert_path") == cert_path
        and payload.get("fields", {}).get("outcome") == outcome
        for payload in payloads
    )

if not has_event(
    passing_cert,
    "range_cert.independent_verify.certified_reduction.chunked_i16",
    "passed",
):
    raise SystemExit("passing certificate CLI event missing from verify-packet.ndjson")

for spec in case_specs:
    case_name, event = spec.split(":", 1)
    cert_path = out_dir / "tampered" / case_name / "range.cert.json"
    if not has_event(cert_path, event, "failed"):
        raise SystemExit(
            f"tampered case {case_name} missing failed CLI event {event}"
        )
PY

echo "range certificate substrate verification complete: cert=$cert_path"
