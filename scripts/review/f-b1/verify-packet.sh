#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

before="$(mktemp)"
after="$(mktemp)"
trap 'rm -f "$before" "$after"' EXIT

profile_dir="${F_B1_PROFILE_DIR:-target/f-b1}"
profile_log="${F_B1_VERIFY_PROFILE_LOG:-$profile_dir/verify-packet-profile.latest.tsv}"
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

profile_phase "hash packet before regen" \
    sh -c 'find docs/review/f-b1 scripts/review/f-b1 -type f -print0 | sort -z | xargs -0 shasum -a 256 > "$1"' sh "$before"
profile_phase "regen packet" scripts/review/f-b1/regen.sh
profile_phase "hash packet after regen" \
    sh -c 'find docs/review/f-b1 scripts/review/f-b1 -type f -print0 | sort -z | xargs -0 shasum -a 256 > "$1"' sh "$after"
profile_phase "compare packet hashes" diff -u "$before" "$after"
profile_phase "check packet git diff" git diff --exit-code -- docs/review/f-b1 scripts/review/f-b1

printf '[f-b1 profile] wrote %s\n' "$profile_log" >&2
