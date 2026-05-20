#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_feedback_fixture_check.sh [--self-test]

RFC §18.10 entrypoint for S5 feedback fixture checks.
Runs H16 feedback-loop policy tests against the committed fixture substrate.
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
      echo "s5_feedback_fixture_check.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  grep -F "verify_s5_h16_feedback_fixture" "$ROOT/gbf-policy/src/s5.rs" >/dev/null
  echo "[S5 FEEDBACK FIXTURE] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --lib s5::tests::f15

cat <<'NOTE'
S5 feedback fixture check PASS substrate=H16 policy tests
SUBSTRATE_ONLY: live feedback consumer producer path is not invoked here yet.
owner: bd-233u / Fit feedback producer integration.
NOTE
