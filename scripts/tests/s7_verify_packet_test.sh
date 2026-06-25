#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

output="$(scripts/review/f-s7/verify-packet.sh --self-test)"

for expected in \
  "S7 verify-packet required gates:" \
  "scripts/s7_preregistration_check.sh" \
  "scripts/s7_preregistration_pin.sh --check-ready" \
  "scripts/review/f-s7/emit-report.py" \
  "s7 validate-closure" \
  "experiments/S7/dense-vs-moe/comparison.json" \
  "docs/experiments/S7-report.md" \
  "S7 verify-packet synthetic S7 CLI feature preflight self-test: ok" \
  "S7 verify-packet synthetic Rust closure gate self-test: ok" \
  "S7 verify-packet synthetic production surface self-test: ok" \
  "S7 verify-packet self-test: ok"
do
  if [[ "$output" != *"$expected"* ]]; then
    echo "verify-packet self-test output missing: $expected" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
done

echo "s7_verify_packet_test: ok"
