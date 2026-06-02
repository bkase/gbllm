#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_pareto_fixture_check.sh [--self-test]

RFC §18.10 entrypoint for S5 Pareto/frontier fixture checks.
Runs the committed frontier fixture recomputation and H14 policy tests.
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
      echo "s5_pareto_fixture_check.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$ROOT/fixtures/s5/frontier/recommendation_a.json"
  test -f "$ROOT/fixtures/s5/frontier/recommendation_b_l_mt4.json"
  test -f "$ROOT/fixtures/s5/frontier/recommendation_b_l_fix1.json"
  test -f "$ROOT/fixtures/s5/frontier/recommendation_tie.json"
  echo "[S5 PARETO FIXTURE] self-test PASS"
  exit 0
fi

cargo test -p gbf-policy --test s5_golden_fixture_corpus s5_frontier_golden_reports_recompute_recommendations
cargo test -p gbf-policy --lib s5::tests::f14

cat <<'NOTE'
S5 Pareto fixture check PASS substrate=frontier fixtures + H14 policy tests
SUBSTRATE_ONLY: live frontier producer emission is not invoked here yet.
owner: bd-9flf/bd-5yfv/bd-3ozo producer integration.
NOTE
