#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_emulator_harness_seed0_check.sh [--self-test]

RFC §18.10 entrypoint for the S5 seed-0 emulator harness check.
Runs the H15 first-commit cardinality fixture and policy tests.
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
      echo "s5_emulator_harness_seed0_check.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$ROOT/fixtures/s5/first_commit/h15/zero_token_payload.bin"
  test -f "$ROOT/fixtures/s5/first_commit/h15/single_charset_v1_token.bin"
  test -f "$ROOT/fixtures/s5/first_commit/h15/two_charset_v1_tokens.bin"
  echo "[S5 EMULATOR HARNESS SEED0] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --test s5_golden_fixture_corpus s5_first_commit_payload_fixtures_cover_h15_cardinality
cargo test -p gbf-policy --lib emulator_harness
cargo test -p gbf-policy --lib s5::tests::h15

cat <<'NOTE'
S5 emulator harness seed-0 check PASS substrate=H15 first-commit fixtures + policy tests
SUBSTRATE_ONLY: live Game Boy emulator harness is not invoked here yet.
owner: bd-3af8 / F-A7 harness producer integration.
NOTE
