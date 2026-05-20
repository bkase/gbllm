#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_reproducibility_smoke.sh [--self-test]

RFC §18.10 entrypoint for the S5 reproducibility smoke. This wrapper delegates
to scripts/s5_replay_verify.sh, which verifies the committed fixtures/s5 replay
corpus and normalized seed-0 log stream.
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
      echo "s5_reproducibility_smoke.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -x "$ROOT/scripts/s5_replay_verify.sh"
  test -f "$ROOT/fixtures/s5/log_streams/seed_0_canonical_run.ndjson"
  echo "[S5 REPRODUCIBILITY SMOKE] self-test PASS"
  exit 0
fi

"$ROOT/scripts/s5_replay_verify.sh"
cat <<'NOTE'
S5 reproducibility smoke PASS substrate=fixtures/s5 replay verifier
SUBSTRATE_ONLY: live two-run producer replay is not invoked here yet.
owner: bd-u4fh.
NOTE
