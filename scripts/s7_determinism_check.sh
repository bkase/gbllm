#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SELF_TEST=0
TEST_ARGS=()

usage() {
  cat <<'USAGE'
Usage: scripts/s7_determinism_check.sh [--self-test] [-- TEST_FILTER_OR_ARGS...]

Runs the focused S7 fixture determinism target for Rep-S7-1..7, D14, O8,
and O9. The Rust target emits the aggregate `s7.determinism.summary` event
over the fixture axes. Full gbf-cli S7 replay/verify-determinism adoption
remains owned by bd-1ryn/bd-2v9r.
USAGE
}

while (($#)); do
  case "$1" in
    --self-test)
      SELF_TEST=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      TEST_ARGS=("$@")
      break
      ;;
    *)
      TEST_ARGS+=("$1")
      shift
      ;;
  esac
done

CMD=(
  cargo test -p gbf-experiments
  --no-default-features --features s7
  --test determinism_s7
)

if ((SELF_TEST == 1)); then
  printf '+'
  printf ' %q' "${CMD[@]}"
  if ((${#TEST_ARGS[@]})); then
    printf ' --'
    printf ' %q' "${TEST_ARGS[@]}"
  fi
  printf '\n'
  echo "s7_determinism_check self-test: ok"
  exit 0
fi

cd "$ROOT"
if ((${#TEST_ARGS[@]})); then
  exec "${CMD[@]}" -- "${TEST_ARGS[@]}"
fi
exec "${CMD[@]}"
