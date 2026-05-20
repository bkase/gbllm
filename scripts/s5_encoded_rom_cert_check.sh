#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_encoded_rom_cert_check.sh [--self-test]

RFC §18.10 entrypoint for S5 EncodedRom certificate checks.
Runs ER-3/ER-7/O11 policy tests and verifies seed-0 fixture certs.
USAGE
}

self_test=0
while (($#)); do
  case "$1" in
    --self-test)
      self_test=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "s5_encoded_rom_cert_check.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$ROOT/fixtures/s5/encoded_rom/seed_0_canonical/build.json"
  test -f "$ROOT/fixtures/s5/encoded_rom/seed_0_canonical/rom.gb.sha256"
  test -d "$ROOT/fixtures/s5/encoded_rom/seed_0_canonical/certs"
  echo "[S5 ENCODED ROM CERT] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --test encoded_rom_identity

cat <<'NOTE'
S5 EncodedRom cert check PASS substrate=seed-0 cert fixtures + ER/O11 policy tests
SUBSTRATE_ONLY: live EncodedRom producer for all five seeds is not invoked here yet.
owner: bd-1d6b.
NOTE
