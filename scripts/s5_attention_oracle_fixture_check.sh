#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_attention_oracle_fixture_check.sh [--self-test]

RFC §18.10 entrypoint for S5 attention-oracle fixture checks.
Runs the oracle contract and binding-hash policy tests.
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
      echo "s5_attention_oracle_fixture_check.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$ROOT/gbf-policy/tests/attention_oracle_contract.rs"
  test -f "$ROOT/gbf-policy/tests/attention_oracle_report_bindings.rs"
  echo "[S5 ATTENTION ORACLE FIXTURE] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --test attention_oracle_contract
cargo test -p gbf-policy --test attention_oracle_report_bindings

cat <<'NOTE'
S5 attention-oracle fixture check PASS substrate=oracle contract + binding-hash tests
SUBSTRATE_ONLY: live gbf-experiments oracle runner is not invoked here yet.
owner: bd-1gmy producer integration.
NOTE
