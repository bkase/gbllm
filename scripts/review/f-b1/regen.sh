#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

export RUST_LOG="${RUST_LOG:-info,gbf=debug}"
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

profile_dir="${F_B1_PROFILE_DIR:-target/f-b1}"
profile_log="${F_B1_PROFILE_LOG:-$profile_dir/regen-profile.latest.tsv}"
mkdir -p "$profile_dir"
printf "phase\tstatus\tseconds\tstarted_at_utc\tended_at_utc\n" > "$profile_log"

profile_phase() {
    local phase="$1"
    shift
    local started_at start_s ended_at end_s status
    started_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    start_s="$(date +%s)"
    printf '[f-b1 profile] start %s\n' "$phase" >&2
    set +e
    "$@"
    status="$?"
    set -e
    ended_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    end_s="$(date +%s)"
    printf "%s\t%s\t%s\t%s\t%s\n" \
        "$phase" "$status" "$((end_s - start_s))" "$started_at" "$ended_at" >> "$profile_log"
    printf '[f-b1 profile] done %s status=%s seconds=%s\n' \
        "$phase" "$status" "$((end_s - start_s))" >&2
    return "$status"
}

profile_phase "gbf-abi compute_shape tests" \
    cargo test -p gbf-abi -- compute_shape::
profile_phase "gbf-verify matmul tests" \
    cargo test -p gbf-verify -- matmul::
profile_phase "gbf-codegen f_b1 tests" \
    cargo test -p gbf-codegen -- f_b1
profile_phase "gbf-runtime f_b1 tests" \
    cargo test -p gbf-runtime -- f_b1
profile_phase "gbf-emu streaming n32 smoke" \
    cargo test -p gbf-emu -- f_b1_l3_streaming_rom_matches_reference_n32
profile_phase "gbf-report realism schema tests" \
    cargo test -p gbf-report -- realism_report_v1
profile_phase "gbf-bench f_b1 tests" \
    cargo test -p gbf-bench -- f_b1
profile_phase "gbf-meta-checks ignored discipline" \
    cargo test -p gbf-meta-checks -- ignored_discipline_heavy_f_b1_tests_are_ignored
profile_phase "gbf-test f_b1_regen artifact sweep" \
    cargo run -p gbf-test --bin f_b1_regen

printf '[f-b1 profile] wrote %s\n' "$profile_log" >&2
