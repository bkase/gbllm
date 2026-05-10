#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
usage: scripts/s1_e2e.sh [--scenario NAME] [--fixture tiny] [--update-goldens]

Runs the F-S1.36 tiny-fixture end-to-end scenario harness. This is an
IntegrationFixture-only gate; it does not run the full TinyStories closure job.
USAGE
}

scenario=""
update_goldens=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scenario)
      if [[ $# -lt 2 ]]; then
        echo "missing value for --scenario" >&2
        usage >&2
        exit 2
      fi
      scenario="$2"
      shift 2
      ;;
    --update-goldens)
      update_goldens=1
      shift
      ;;
    --fixture)
      if [[ $# -lt 2 || "$2" != "tiny" ]]; then
        echo "F-S1.36 only supports --fixture tiny" >&2
        usage >&2
        exit 2
      fi
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

export RUST_TEST_THREADS=1
if [[ -n "$scenario" ]]; then
  export S1_E2E_SCENARIO="$scenario"
fi
if [[ "$update_goldens" -eq 1 ]]; then
  export GBF_UPDATE_GOLDENS=1
fi

start_seconds=$(date +%s)
cargo test -p gbf-experiments --test e2e -- --nocapture
end_seconds=$(date +%s)

if [[ -n "$scenario" ]]; then
  echo "[S1 E2E] PASS scenario=$scenario runtime_seconds=$((end_seconds - start_seconds))"
else
  echo "[S1 E2E] PASS scenarios=all runtime_seconds=$((end_seconds - start_seconds))"
fi
