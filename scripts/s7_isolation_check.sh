#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PASS_VERSION="${S7_PASS_VERSION:-}"
SEED_LIST="${S7_SEED_LIST:-0,1,2,3,4}"
DRY_RUN=0
SELF_TEST=0
FIXTURE_MODE=1

usage() {
  cat <<'USAGE'
Usage: scripts/s7_isolation_check.sh [--fixture|--live-cli] [--pass-version VERSION] [--seed-list LIST] [--dry-run] [--self-test]

By default, runs the focused S7 fixture isolation tests that are available
before the split gbf-cli S7 replay gates land. With --live-cli, runs the S7
replay isolation commands in the RFC-mandated split-feature order:
  1. gbf-cli with --features s7-moe for topology MoeTiny
  2. gbf-cli with --features s7-dense-matched for topology MoeTinyDenseMatched

Live execution depends on the gbf-cli S7 feature gates owned by bd-1ryn.
Use --dry-run or --self-test to validate command shape until those gates land.

Options:
  --fixture               run focused fixture isolation tests (default)
  --live-cli              run the future split-feature gbf-cli replay path
  --pass-version VERSION  pass_version pinned in the final S7 report
  --seed-list LIST        comma-separated seed list (default: 0,1,2,3,4)
  --dry-run               print commands without executing them
  --self-test             validate command shape/order without executing cargo
USAGE
}

while (($#)); do
  case "$1" in
    --fixture)
      FIXTURE_MODE=1
      shift
      ;;
    --live-cli)
      FIXTURE_MODE=0
      shift
      ;;
    --pass-version)
      if (($# < 2)); then
        echo "error: --pass-version requires a value" >&2
        exit 2
      fi
      PASS_VERSION="$2"
      shift 2
      ;;
    --seed-list)
      if (($# < 2)); then
        echo "error: --seed-list requires a value" >&2
        exit 2
      fi
      SEED_LIST="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --self-test)
      SELF_TEST=1
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

print_command() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
}

build_replay_cmd() {
  local feature="$1"
  local topology="$2"

  CMD=(
    cargo run --release -p gbf-cli --features "$feature" -- s7 replay
    --gutenberg-manifest fixtures/corpora/gutenberg.toml
    --charset fixtures/charsets/charset_v1.toml
    --matched-bytes experiments/S7/profile/matched_bytes.json
    --pass-version "$PASS_VERSION"
    --topology "$topology"
    --seed-list "$SEED_LIST"
    --device-profile S7CpuDeterministic
  )
}

run_replay() {
  local feature="$1"
  local topology="$2"

  build_replay_cmd "$feature" "$topology"
  print_command "${CMD[@]}"
  if ((DRY_RUN == 0)); then
    "${CMD[@]}"
  fi
}

run_all() {
  cd "$ROOT"
  run_replay s7-moe MoeTiny
  run_replay s7-dense-matched MoeTinyDenseMatched
}

line_number_for() {
  local text="$1"
  local pattern="$2"
  printf '%s\n' "$text" | awk -v pattern="$pattern" 'index($0, pattern) { print NR; exit }'
}

run_self_test() {
  PASS_VERSION="${PASS_VERSION:-self-test-pass-version}"

  local output
  output="$(run_all)"

  local moe_line
  local dense_line
  moe_line="$(line_number_for "$output" "--features s7-moe")"
  dense_line="$(line_number_for "$output" "--features s7-dense-matched")"

  if [[ -z "$moe_line" || -z "$dense_line" ]]; then
    echo "error: self-test did not find both split-feature replay commands" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  if ((moe_line >= dense_line)); then
    echo "error: self-test found dense replay before MoE replay" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  if [[ "$output" != *"--topology MoeTiny"* ]]; then
    echo "error: self-test did not find MoeTiny topology" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  if [[ "$output" != *"--topology MoeTinyDenseMatched"* ]]; then
    echo "error: self-test did not find MoeTinyDenseMatched topology" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi

  printf '%s\n' "$output"
  echo "s7_isolation_check self-test: ok"
}

if ((SELF_TEST == 1)); then
  run_self_test
  exit 0
fi

run_fixture_isolation() {
  cd "$ROOT"
  cargo test -p gbf-experiments --no-default-features --features s7 \
    --test determinism_s7 -- o8_fixture_replay_ignores_disallowed_environment_inputs --exact
  cargo test -p gbf-experiments --no-default-features --features s7 \
    --test determinism_s7 -- o9_fixture_run_order_preserves_per_seed_hashes --exact
}

if ((FIXTURE_MODE == 1)); then
  run_fixture_isolation
  exit 0
fi

if [[ -z "$PASS_VERSION" ]]; then
  echo "error: pass version is required; pass --pass-version or set S7_PASS_VERSION" >&2
  exit 2
fi

run_all
