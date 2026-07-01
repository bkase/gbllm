#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-verify-packet-test.XXXXXX)"
dirty_probe="experiments/S7/verify-packet-dirty-probe.tmp"
rm -f "$dirty_probe"
trap 'rm -f "$dirty_probe"; rm -rf "$tmp"' EXIT

output="$(scripts/review/f-s7/verify-packet.sh --self-test)"

for expected in \
  "S7 verify-packet required gates:" \
  "scripts/s7_preregistration_check.sh" \
  "scripts/s7_preregistration_pin.sh --check-ready" \
  "scripts/review/f-s7/emit-report.py" \
  "scripts/review/f-s7/assemble-packet.py --manifest <production-bundle-manifest.json> --run-reviews" \
  "scripts/review/f-s7/audit-production-runner.py" \
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

printf 'dirty packet probe\n' >"$dirty_probe"
if scripts/review/f-s7/verify-packet.sh --skip-gates >"$tmp/dirty.out" 2>&1; then
  echo "expected production verify-packet to reject a dirty git worktree" >&2
  exit 1
fi
rg -n "S7 production closure verification requires clean git worktree before review/head validation" \
  "$tmp/dirty.out" >/dev/null
rm -f "$dirty_probe"

echo "s7_verify_packet_test: ok"
