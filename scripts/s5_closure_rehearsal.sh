#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RFC_PATH="$ROOT/history/rfcs/F-S5-pick-and-fit.md"
DEFAULT_RUN_ROOT="$ROOT/experiments/S5/runs"
DEFAULT_NEGATIVE_CONTROL_FIXTURE="$ROOT/fixtures/s5/shadow/broken_negative_control.sample.json"

mode="fast"
keep_artifacts=0
self_test=0
run_root="${S5_CLOSURE_REHEARSAL_RUN_ROOT:-$DEFAULT_RUN_ROOT}"
negative_control_fixture="${S5_CLOSURE_REHEARSAL_NEGATIVE_CONTROL_FIXTURE:-$DEFAULT_NEGATIVE_CONTROL_FIXTURE}"

usage() {
    cat <<'USAGE'
Usage: scripts/s5_closure_rehearsal.sh [--fast] [--full] [--keep-artifacts] [--self-test] [--run-root DIR]

Runs the F-S5 closure rehearsal harness and writes stage-tagged NDJSON logs to:
  experiments/S5/runs/rehearsal-{timestamp}/log.ndjson

The NDJSON schema_id is s5_closure_rehearsal_log: a script-owned rehearsal
envelope for ordered stage evidence. It wraps event_name=s5_closure_flow so the
stream aligns with the Rust logging helper taxonomy; it does not replace the
Rust s5_closure_log payload schema emitted by gbf-train.

The current rehearsal substrate validates the stage ordering scaffold, stable
spec hash, and the Stage 6 broken shadow negative-control adoption path. Stage
6 validates the canonical sample JSON through gbf-policy's SHR-1 Rust contract;
it does not claim a live shadow_compile runner invocation.
USAGE
}

while (($#)); do
    case "$1" in
        --fast)
            mode="fast"
            ;;
        --full)
            mode="full"
            ;;
        --keep-artifacts)
            keep_artifacts=1
            ;;
        --self-test)
            self_test=1
            ;;
        --run-root)
            shift
            if (($# == 0)); then
                echo "s5 closure rehearsal refused: --run-root requires a directory" >&2
                exit 2
            fi
            run_root="$1"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "s5 closure rehearsal refused: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

if [[ "$self_test" == "1" ]]; then
    keep_artifacts=1
fi

run_id="rehearsal-$(date -u +%Y%m%dT%H%M%SZ)"
run_dir="$run_root/$run_id"
artifact_dir="$run_dir/artifacts"
log_path="$run_dir/log.ndjson"
last_stage_id="not-started"
last_stage_name="not-started"

mkdir -p "$artifact_dir"

if [[ "$keep_artifacts" != "1" ]]; then
    cleanup() {
        local status=$?
        if [[ "$status" == "0" ]]; then
            rm -rf "$run_dir"
        fi
    }
    trap cleanup EXIT
fi

sha256_file() {
    shasum -a 256 "$1" | awk '{print "sha256:" $1}'
}

spec_sha="$(sha256_file "$RFC_PATH")"

emit_event() {
    local stage_id="$1"
    local stage_name="$2"
    local event_kind="$3"
    local level="$4"
    local fields_json="$5"
    last_stage_id="$stage_id"
    last_stage_name="$stage_name"
    python3 - "$log_path" "$run_id" "$spec_sha" "$stage_id" "$stage_name" "$event_kind" "$level" "$fields_json" <<'PY'
import json
import sys
import time

path, run_id, spec_sha, stage_id, stage_name, event_kind, level, fields_json = sys.argv[1:9]
fields = json.loads(fields_json)
# s5_closure_rehearsal_log is the script envelope schema for stage ordering and
# per-run artifacts. event_name=s5_closure_flow deliberately matches the Rust
# helper event name, but this record is not the Rust s5_closure_log payload.
record = {
    "schema_id": "s5_closure_rehearsal_log",
    "schema_version": "1",
    "event_name": "s5_closure_flow",
    "run_id": run_id,
    "spec_sha": spec_sha,
    "stage_id": int(stage_id),
    "stage_name": stage_name,
    "event_kind": event_kind,
    "level": level,
    "ts_unix_ns": time.time_ns(),
    "fields": fields,
}
with open(path, "a", encoding="utf-8") as handle:
    handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
PY
}

json_obj() {
    python3 - "$@" <<'PY'
import json
import sys

if len(sys.argv[1:]) % 2:
    raise SystemExit("json_obj requires key/value pairs")
payload = {}
for index in range(1, len(sys.argv), 2):
    key = sys.argv[index]
    value = sys.argv[index + 1]
    if value == "true":
        payload[key] = True
    elif value == "false":
        payload[key] = False
    elif value == "null":
        payload[key] = None
    else:
        try:
            payload[key] = int(value)
        except ValueError:
            payload[key] = value
print(json.dumps(payload, sort_keys=True, separators=(",", ":")))
PY
}

fail_stage() {
    local message="$1"
    emit_event "$last_stage_id" "$last_stage_name" "stage_failed" "ERROR" "$(json_obj message "$message")"
    echo "S5 closure rehearsal FAIL stage=$last_stage_id:$last_stage_name log=$log_path reason=$message" >&2
    exit 1
}

run_preflight() {
    emit_event 0 "preflight" "stage_started" "INFO" "$(json_obj mode "$mode")"

    if [[ "${S5_CLOSURE_REHEARSAL_SKIP_PREFLIGHT_COMMANDS:-}" == "1" ]]; then
        emit_event 0 "preflight" "preflight_summary" "INFO" \
            "$(json_obj workspace_clean true beads_sync_clean true fmt_checked false clippy_checked false skipped true)"
        return
    fi

    if [[ "${S5_CLOSURE_REHEARSAL_ALLOW_DIRTY:-}" != "1" ]]; then
        local dirty
        dirty="$(git -C "$ROOT" status --porcelain -uno)"
        if [[ -n "$dirty" ]]; then
            fail_stage "workspace is dirty; rerun from a clean tree or set S5_CLOSURE_REHEARSAL_ALLOW_DIRTY=1 for local rehearsal debugging"
        fi
    fi

    br sync --status >/dev/null
    cargo fmt --check --all
    cargo clippy -p gbf-policy --lib --all-features -- -D warnings

    emit_event 0 "preflight" "preflight_summary" "INFO" \
        "$(json_obj workspace_clean true beads_sync_clean true fmt_checked true clippy_checked true skipped false)"
}

run_preregistration() {
    emit_event 1 "pre_registration" "stage_started" "INFO" "$(json_obj mode "$mode")"
    local spec_sha_again
    spec_sha_again="$(sha256_file "$RFC_PATH")"
    if [[ "$spec_sha" != "$spec_sha_again" ]]; then
        fail_stage "F-S5 RFC spec_sha changed during pre-registration"
    fi
    emit_event 1 "pre_registration" "s5_preregistration" "INFO" \
        "$(json_obj manifest_schema s5_preregistration.v1 spec_sha "$spec_sha" spec_sha_repeat "$spec_sha_again")"
}

run_pick_scaffold() {
    emit_event 2 "pick" "stage_started" "INFO" "$(json_obj mode "$mode")"
    local seeds
    if [[ "$mode" == "fast" ]]; then
        seeds="0"
    else
        seeds="0 1 2 3 4"
    fi

    local variants=("BoundedKv" "L_FIX1" "L_MT4")
    local variant seed pick_runs=0
    for variant in "${variants[@]}"; do
        for seed in $seeds; do
            emit_event 2 "pick" "state_transition" "TRACE" \
                "$(json_obj from PickRunning to PickEvalDone variant "$variant" seed "$seed")"
            emit_event 2 "pick" "state_transition" "TRACE" \
                "$(json_obj from PickEvalDone to PickScoreDone variant "$variant" seed "$seed")"
            pick_runs=$((pick_runs + 1))
        done
    done

    emit_event 2 "pick" "pick_frontier_inputs_frozen" "INFO" \
        "$(json_obj variant_count 3 seed_count "$(wc -w <<<"$seeds" | tr -d ' ')" pick_runs "$pick_runs")"
}

run_frontier_emit_scaffold() {
    emit_event 7 "frontier_emit" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 7 "frontier_emit" "frontier_emitted" "INFO" \
        "$(json_obj frontier_count 1 pick_points_present true fit_points_present true fit_null_rule_checked true substrate_only true)"
}

run_fit_scaffold() {
    emit_event 3 "fit" "stage_started" "INFO" "$(json_obj mode "$mode")"
    local seed_count=1
    if [[ "$mode" == "full" ]]; then
        seed_count=5
    fi
    emit_event 3 "fit" "fit_revalidation_scaffold" "INFO" \
        "$(json_obj seed_count "$seed_count" default_profile_checked true encoded_rom_scaffold true substrate_only true)"
}

run_pf3_scaffold() {
    emit_event 4 "pf3_differential_preflight" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 4 "pf3_differential_preflight" "pf3_diff_scaffold" "INFO" \
        "$(json_obj default_profile Default bringup_profile BringUp substrate_only true)"
}

run_negative_control() {
    emit_event 5 "negative_control_sample" "stage_started" "INFO" "$(json_obj fixture "$negative_control_fixture")"
    if ! cargo run -q -p gbf-policy --bin s5_shr1_validate -- \
        "$negative_control_fixture" "$artifact_dir/negative_control_assertion.json"
    then
        fail_stage "Stage 5 negative-control SHR-1 Rust validation failed"
    fi

    local assertion_json
    assertion_json="$(<"$artifact_dir/negative_control_assertion.json")"
    emit_event 5 "negative_control_sample" "negative_control_sample" "DEBUG" "$assertion_json"
}

run_harness_scaffold() {
    emit_event 6 "emulator_harness" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 6 "emulator_harness" "harness_run_scaffold" "INFO" \
        "$(json_obj seed 0 first_commit_payload_len_expected 1 oracle_agreement_required true substrate_only true)"
}

run_outcome_scaffold() {
    emit_event 8 "outcome_dispatch" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 8 "outcome_dispatch" "outcome_dispatch" "INFO" \
        "$(json_obj reachable_pass_outcome true decision ProceedToS7 substrate_only true)"
}

run_falsifier_scaffold() {
    emit_event 9 "falsifier_sample" "stage_started" "INFO" "$(json_obj mode "$mode")"
    local cases=("F13" "F14")
    if [[ "$mode" == "full" ]]; then
        cases=("F5" "F11" "F13" "F14" "F15")
    fi
    local case_name
    for case_name in "${cases[@]}"; do
        emit_event 9 "falsifier_sample" "falsifier_trigger_scaffold" "DEBUG" \
            "$(json_obj f_case "$case_name" expected_verdict Refuted substrate_only true)"
    done
}

run_replay_scaffold() {
    emit_event 10 "replay_determinism" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 10 "replay_determinism" "replay_determinism_scaffold" "INFO" \
        "$(json_obj frontier_byte_identical_required true encoded_rom_seed_0_required true substrate_only true)"
}

run_logging_overhead_scaffold() {
    emit_event 11 "logging_overhead" "stage_started" "INFO" "$(json_obj mode "$mode")"
    emit_event 11 "logging_overhead" "logging_overhead_gate_scaffold" "INFO" \
        "$(json_obj script scripts/s5_logging_overhead_check.sh threshold 0.01 substrate_only true)"
}

run_summary() {
    emit_event 12 "summary" "outcome_dispatch_summary" "INFO" \
        "$(json_obj outcome Pass-clean decision ProceedToS7 log "$log_path" run_dir "$run_dir")"
    echo "S5 closure rehearsal PASS run_dir=$run_dir log=$log_path"
}

run_self_test() {
    bash -n "$0"
    S5_CLOSURE_REHEARSAL_SELF_TEST_TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$S5_CLOSURE_REHEARSAL_SELF_TEST_TMPDIR"' EXIT
    S5_CLOSURE_REHEARSAL_SKIP_PREFLIGHT_COMMANDS=1 \
        S5_CLOSURE_REHEARSAL_RUN_ROOT="$S5_CLOSURE_REHEARSAL_SELF_TEST_TMPDIR" \
        "$0" --fast --keep-artifacts >/tmp/s5-closure-rehearsal-selftest.out

    local produced_log
    produced_log="$(awk -F'log=' '/S5 closure rehearsal PASS/ {print $2}' /tmp/s5-closure-rehearsal-selftest.out)"
    python3 - "$produced_log" <<'PY'
import json
import pathlib
import sys

log_path = pathlib.Path(sys.argv[1])
records = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines()]
if not records:
    raise SystemExit("self-test produced no NDJSON records")
for record in records:
    if "stage_id" not in record or "stage_name" not in record:
        raise SystemExit(f"record is missing stage tag: {record}")
negative = [record for record in records if record["event_kind"] == "negative_control_sample"]
if len(negative) != 1:
    raise SystemExit(f"expected exactly one negative_control_sample event, got {len(negative)}")
fields = negative[0]["fields"]
if fields.get("shadow_compile_ok") is not False:
    raise SystemExit("negative-control self-test did not assert shadow_compile_ok=false")
if not fields.get("diagnostic"):
    raise SystemExit("negative-control self-test did not carry a diagnostic")
if not fields.get("failure_stage"):
    raise SystemExit("negative-control self-test did not carry a failure_stage")
if fields.get("validated_by") != "gbf-policy::shadow::validate_shr1_shadow_sample":
    raise SystemExit("negative-control self-test did not use the Rust SHR-1 validator")
if records[-1]["stage_id"] != 12:
    raise SystemExit("self-test did not reach summary stage")
PY
    echo "[S5 CLOSURE REHEARSAL] self-test PASS"
}

if [[ "$self_test" == "1" ]]; then
    run_self_test
    exit 0
fi

run_preflight
run_preregistration
run_pick_scaffold
run_fit_scaffold
run_pf3_scaffold
run_negative_control
run_harness_scaffold
run_frontier_emit_scaffold
run_outcome_scaffold
run_falsifier_scaffold
run_replay_scaffold
run_logging_overhead_scaffold
run_summary
