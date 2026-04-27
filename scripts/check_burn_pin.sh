#!/usr/bin/env bash
# Verify Burn stays an exact, gbf-train-owned training dependency.
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/check_burn_pin.sh [--static-only|--self-test]

Checks:
  - workspace Cargo.toml pins burn with an exact "=..." version
  - gbf-train consumes the workspace burn dependency behind burn-adapter
  - gbf-model has no burn dependency, including package aliases
  - gbf-model/src has no direct burn imports/references
  - unless --static-only is set, the repo architecture Burn boundary test passes
  - unless --static-only is set, a one-step Burn QAT train/export round trip passes

Set GBF_BURN_PIN_REPO_ROOT to test a fixture repository root.
USAGE
}

mode=check
while (($#)); do
    case "$1" in
        --static-only)
            mode=static
            ;;
        --self-test)
            mode=self_test
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
    shift
done

script_repo_root="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"

if [[ -n "${GBF_BURN_PIN_REPO_ROOT:-}" ]]; then
    repo_root="$GBF_BURN_PIN_REPO_ROOT"
else
    repo_root="$script_repo_root"
fi

fail() {
    echo "Burn pin check failed: $*" >&2
    exit 1
}

run_static_checks() {
    if ! (
        cd "$script_repo_root"
        GBF_BURN_PIN_REPO_ROOT="$repo_root" cargo test -p gbf-train \
            adapter::tests::burn_static_boundary_from_env_root_passes -- --exact
    ); then
        fail "static Burn boundary test failed"
    fi
}

run_self_test() {
    if ! (
        cd "$script_repo_root"
        cargo test -p gbf-train \
            adapter::tests::burn_static_boundary_rejects_invalid_fixtures -- --exact
    ); then
        fail "Burn pin self-test failed"
    fi
}

case "$mode" in
    self_test)
        run_self_test
        ;;
    static)
        run_static_checks
        ;;
    check)
        run_static_checks
        (
            cd "$script_repo_root"
            cargo test -p gbf-test --test architecture \
                burn_imports_are_confined_to_train_adapter -- --exact
            cargo test -p gbf-train --features burn-adapter \
                qat::ternary::tests::burn_ternary_one_step_train_export_round_trip_survives_burn_api \
                -- --exact
        )
        ;;
    *)
        fail "unknown mode: $mode"
        ;;
esac
