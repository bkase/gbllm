#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
usage: scripts/s1_e2e_cli.sh [--scenario NAME] [--fixture tiny] [--out-dir DIR]

Runs the CLI-backed F-S1 IntegrationFixture producer path:
baseline -> 5-seed replay -> score -> negative-test -> ablation -> oracle -> report.

This script is tiny-fixture only. Full TinyStories closure artifacts remain owned
by the full S1 closure path, not this PR-cycle producer adoption.

Supported scenarios:
  pass_clean
  pass_with_warning
  fail_substrate_nan
  fail_substrate_zero_grad
  fail_capacity_toytiny
  fail_suspicious_low_bpc
  fail_phase_ternary_leak
  fail_metric_modulo_shuffle
USAGE
}

scenario="pass_clean"
out_dir=""

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
    --fixture)
      if [[ $# -lt 2 || "$2" != "tiny" ]]; then
        echo "bd-1vy9 only supports --fixture tiny" >&2
        usage >&2
        exit 2
      fi
      shift 2
      ;;
    --out-dir)
      if [[ $# -lt 2 ]]; then
        echo "missing value for --out-dir" >&2
        usage >&2
        exit 2
      fi
      out_dir="$2"
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

case "$scenario" in
  pass_clean|pass_with_warning|fail_substrate_nan|fail_substrate_zero_grad|fail_capacity_toytiny|fail_suspicious_low_bpc|fail_phase_ternary_leak|fail_metric_modulo_shuffle)
    ;;
  *)
    echo "unsupported S1 E2E CLI scenario: $scenario" >&2
    usage >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml"
zero_sha="sha256:0000000000000000000000000000000000000000000000000000000000000000"

if [[ -z "$out_dir" ]]; then
  out_dir="$(mktemp -d "${TMPDIR:-/tmp}/gbf-s1-e2e-cli.XXXXXX")"
fi
mkdir -p "$out_dir"

export BURN_NDARRAY_NUM_THREADS=1
export BURN_DETERMINISTIC=1
export OMP_NUM_THREADS=1
export RAYON_NUM_THREADS=1

if [[ -n "${GBF_BIN:-}" ]]; then
  gbf_bin="$GBF_BIN"
else
  cargo_build_args=(build -p gbf-cli)
  case "$scenario" in
    fail_substrate_nan|fail_substrate_zero_grad|fail_capacity_toytiny|fail_suspicious_low_bpc|fail_phase_ternary_leak|fail_metric_modulo_shuffle)
      cargo_build_args+=(--features falsify)
      ;;
  esac
  cargo "${cargo_build_args[@]}" >/dev/null
  gbf_bin="$repo_root/target/debug/gbf-cli"
fi

run_gbf() {
  env -i \
    BURN_NDARRAY_NUM_THREADS=1 \
    BURN_DETERMINISTIC=1 \
    OMP_NUM_THREADS=1 \
    RAYON_NUM_THREADS=1 \
    "$gbf_bin" "$@"
}

seed_list="0,1,2,3,4"
replay_extra=()
expected_replay_failure=0
case "$scenario" in
  fail_substrate_nan)
    seed_list="0"
    replay_extra+=(--inject-non-finite-loss-at-step 3)
    expected_replay_failure=1
    ;;
  fail_substrate_zero_grad)
    replay_extra+=(--zero-gradients)
    ;;
esac

run_gbf s1 fit-baseline \
  --manifest "$manifest" \
  --seed 0 > "$out_dir/s1_baseline.v1.json"

set +e
run_gbf s1 replay \
  --manifest "$manifest" \
  --pass-version 0.1.0 \
  --seed-list "$seed_list" \
  --device-profile S1CpuDeterministic \
  --budget-profile integration-fixture \
  --allow-noncanonical-integration-fixture \
  --out-dir "$out_dir" \
  "${replay_extra[@]}" > "$out_dir/replay_summary.json"
replay_status=$?
set -e

if [[ "$expected_replay_failure" -eq 1 ]]; then
  if [[ "$replay_status" -eq 0 ]]; then
    echo "scenario $scenario expected replay to fail through the falsify substitute" >&2
    exit 1
  fi
else
  if [[ "$replay_status" -ne 0 ]]; then
    echo "scenario $scenario replay failed unexpectedly" >&2
    exit "$replay_status"
  fi
fi

if [[ "$scenario" != "fail_substrate_nan" ]]; then
  score_extra=()
  if [[ "$scenario" == "fail_suspicious_low_bpc" ]]; then
    score_extra+=(--fixture-forced-bpc 0.25)
  fi
  if [[ "$scenario" == "fail_capacity_toytiny" ]]; then
    score_extra+=(--fixture-forced-bpc 9.0)
  fi

  for seed in 0 1 2 3 4; do
    mkdir -p "$out_dir/seed-$seed"
    run_gbf s1 score \
      --manifest "$manifest" \
      --seed "$seed" \
      --checkpoint-sha "$zero_sha" \
      --fixture-uniform-scorer \
      "${score_extra[@]}" > "$out_dir/seed-$seed/s1_score.v1.json"
  done
fi

mkdir -p "$out_dir/seed-0"

run_gbf s1 negative-test \
  --manifest "$manifest" \
  --seed 0 \
  --checkpoint-sha "$zero_sha" \
  --fixture-uniform-scorer > "$out_dir/seed-0/s1_negative_test.v1.json"

ablation_extra=()
if [[ "$scenario" == "fail_phase_ternary_leak" ]]; then
  ablation_extra+=(--fixture-mismatch)
fi
run_gbf s1 ablation \
  --fixture-self-compare \
  "${ablation_extra[@]}" > "$out_dir/seed-0/s1_ablation.v1.json"

if [[ "$scenario" == "fail_metric_modulo_shuffle" ]]; then
  run_gbf s1 oracle \
    --manifest "$manifest" \
    --seed 0 \
    --fixture-fail-o-metric-4 > "$out_dir/s1_oracle.v1.json"
else
  run_gbf s1 oracle \
    --manifest "$manifest" \
    --seed 0 > "$out_dir/s1_oracle.v1.json"
fi

run_gbf s1 report \
  --fixture-scenario "$(tr '_' '-' <<<"$scenario")" \
  --artifact-dir "$out_dir" > "$out_dir/report_summary.json"

echo "[S1 E2E CLI] PASS scenario=$scenario out_dir=$out_dir"
