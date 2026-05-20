#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_FIXTURE="$ROOT/fixtures/s5/chrome_budget/real_canonical.toml"
SYNTHETIC_FIXTURE="$ROOT/fixtures/s5/chrome_budget/synthetic_canonical.toml"

usage() {
  cat <<'USAGE'
Usage: scripts/check-nucleus-drift.sh [--self-test]

RFC §18.10 entrypoint for the S5 runtime_nucleus_hash drift gate.
Until the live gbf-runtime nucleus emitter is wired, this wrapper verifies the
committed D10/D18 substrate fixtures and RV policy tests, and prints the owner
that must replace the fixture substrate with a live producer check.
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
      echo "check-nucleus-drift.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$REAL_FIXTURE"
  test -f "$SYNTHETIC_FIXTURE"
  grep -F 'runtime_nucleus_hash = "sha256:' "$REAL_FIXTURE" >/dev/null
  grep -F 'runtime_nucleus_hash = "SYNTHETIC_REFERENCE:' "$SYNTHETIC_FIXTURE" >/dev/null
  echo "[S5 NUCLEUS DRIFT] self-test PASS"
  exit 0
fi

grep -F 'runtime_nucleus_hash = "sha256:' "$REAL_FIXTURE" >/dev/null
grep -F 'runtime_nucleus_hash = "SYNTHETIC_REFERENCE:' "$SYNTHETIC_FIXTURE" >/dev/null
cargo test -p gbf-policy --test re_validation

cat <<'NOTE'
S5 nucleus drift gate PASS substrate=fixtures/s5/chrome_budget + gbf-policy::re_validation
SUBSTRATE_ONLY: live gbf-runtime nucleus emission is not invoked here yet.
owner: gbf-runtime / legacy bd-177-style runtime_nucleus_hash CI drift producer.
NOTE
