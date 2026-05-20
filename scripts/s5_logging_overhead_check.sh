#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE_FEATURES="s5-no-log,qat,burn-adapter"
INSTRUMENTED_FEATURES="qat,burn-adapter"
BIN_NAME="s5_logging_overhead_workload"
WARMUP_ITERATIONS=5
MEASURED_ITERATIONS=50
THRESHOLD="0.01"

usage() {
    cat <<'USAGE'
Usage: scripts/s5_logging_overhead_check.sh [--check-env-only|--self-test]

Runs the D14 logging-overhead gate for the tiny preflight + shadow_compile
workload. The script refuses to run unless benchmark controls are explicit:
  - CPU governor fixed to performance or userspace
  - OMP_NUM_THREADS=1
  - RAYON_NUM_THREADS=1
  - BURN_NDARRAY_NUM_THREADS=1
  - warm cache after warmup
  - baseline/instrumented runs interleaved by pair

On Linux, CPU governor is read from cpufreq sysfs. On hosts without cpufreq
sysfs, set S5_LOGGING_OVERHEAD_CPU_GOVERNOR=performance or userspace after
pinning the host externally.
USAGE
}

mode=run
while (($#)); do
    case "$1" in
        --check-env-only)
            mode=check_env
            ;;
        --self-test)
            mode=self_test
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "D14 logging-overhead refused: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

refuse() {
    echo "D14 logging-overhead refused: $*" >&2
    return 2
}

require_env_exact() {
    local name="$1"
    local expected="$2"
    local observed="${!name-}"
    if [[ "$observed" != "$expected" ]]; then
        refuse "$name must be $expected for single-worker measurement_env control (observed: ${observed:-<unset>})"
        return $?
    fi
}

cpu_governor_control() {
    if [[ "${S5_LOGGING_OVERHEAD_SELF_TEST_NO_SYSFS:-}" == "1" ]]; then
        printf '%s\n' "${S5_LOGGING_OVERHEAD_CPU_GOVERNOR:-}"
        return 0
    fi

    local governors=()
    local path
    shopt -s nullglob
    for path in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
        governors+=("$(<"$path")")
    done
    shopt -u nullglob

    if ((${#governors[@]} == 0)); then
        printf '%s\n' "${S5_LOGGING_OVERHEAD_CPU_GOVERNOR:-}"
        return 0
    fi

    printf '%s\n' "${governors[@]}" | sort -u | paste -sd, -
}

validate_measurement_env() {
    require_env_exact OMP_NUM_THREADS 1 || return $?
    require_env_exact RAYON_NUM_THREADS 1 || return $?
    require_env_exact BURN_NDARRAY_NUM_THREADS 1 || return $?

    local governor
    governor="$(cpu_governor_control)"
    case "$governor" in
        performance|userspace)
            ;;
        "")
            refuse "CPU governor control is absent; fix the governor to performance/userspace or set S5_LOGGING_OVERHEAD_CPU_GOVERNOR after external pinning"
            return $?
            ;;
        *)
            refuse "CPU governor must be fixed to performance or userspace (observed: $governor)"
            return $?
            ;;
    esac
}

json_number_array() {
    python3 - "$@" <<'PY'
import json
import sys
print(json.dumps([int(value) for value in sys.argv[1:]], separators=(",", ":")))
PY
}

median_from_args() {
    python3 - "$@" <<'PY'
import statistics
import sys
print(int(statistics.median(int(value) for value in sys.argv[1:])))
PY
}

extract_workload_json_field() {
    local field="$1"
    local payload
    payload="$(cat)"
    python3 - "$field" "$payload" <<'PY'
import json
import sys

field = sys.argv[1]
value = json.loads(sys.argv[2])[field]
if isinstance(value, bool):
    print("true" if value else "false")
else:
    print(value)
PY
}

run_one() {
    local bin="$1"
    "$bin" --warmup "$WARMUP_ITERATIONS" --measured 1
}

validate_logging_compiled_out() {
    local kind="$1"
    local observed="$2"
    local expected="$3"
    if [[ "$observed" != "$expected" ]]; then
        refuse "$kind workload reported logging_compiled_out=$observed; expected $expected"
        return $?
    fi
}

run_gate() {
    validate_measurement_env || exit $?

    local tmpdir
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    (
        cd "$ROOT"
        cargo build --release -p gbf-train --bin "$BIN_NAME" --no-default-features --features "$BASELINE_FEATURES"
        cp "target/release/$BIN_NAME" "$tmpdir/baseline"
        cargo build --release -p gbf-train --bin "$BIN_NAME" --no-default-features --features "$INSTRUMENTED_FEATURES"
        cp "target/release/$BIN_NAME" "$tmpdir/instrumented"
    )

    local baseline_samples=()
    local instrumented_samples=()
    local i baseline_output instrumented_output baseline_ns instrumented_ns baseline_compiled_out instrumented_compiled_out
    for ((i = 1; i <= MEASURED_ITERATIONS; i++)); do
        if ((i % 2 == 1)); then
            baseline_output="$(run_one "$tmpdir/baseline")"
            instrumented_output="$(run_one "$tmpdir/instrumented")"
        else
            instrumented_output="$(run_one "$tmpdir/instrumented")"
            baseline_output="$(run_one "$tmpdir/baseline")"
        fi
        baseline_ns="$(printf '%s\n' "$baseline_output" | extract_workload_json_field median_ns)"
        instrumented_ns="$(printf '%s\n' "$instrumented_output" | extract_workload_json_field median_ns)"
        baseline_compiled_out="$(printf '%s\n' "$baseline_output" | extract_workload_json_field logging_compiled_out)"
        instrumented_compiled_out="$(printf '%s\n' "$instrumented_output" | extract_workload_json_field logging_compiled_out)"
        validate_logging_compiled_out baseline "$baseline_compiled_out" true || exit $?
        validate_logging_compiled_out instrumented "$instrumented_compiled_out" false || exit $?
        baseline_samples+=("$baseline_ns")
        instrumented_samples+=("$instrumented_ns")
    done

    local median_baseline_ns median_instrumented_ns baseline_json instrumented_json governor
    median_baseline_ns="$(median_from_args "${baseline_samples[@]}")"
    median_instrumented_ns="$(median_from_args "${instrumented_samples[@]}")"
    baseline_json="$(json_number_array "${baseline_samples[@]}")"
    instrumented_json="$(json_number_array "${instrumented_samples[@]}")"
    governor="$(cpu_governor_control)"

    python3 - \
        "$median_baseline_ns" \
        "$median_instrumented_ns" \
        "$THRESHOLD" \
        "$BASELINE_FEATURES" \
        "$INSTRUMENTED_FEATURES" \
        "$WARMUP_ITERATIONS" \
        "$MEASURED_ITERATIONS" \
        "$governor" \
        "$baseline_json" \
        "$instrumented_json" <<'PY'
import json
import sys

median_baseline_ns = int(sys.argv[1])
median_instrumented_ns = int(sys.argv[2])
threshold = float(sys.argv[3])
baseline_features = sys.argv[4]
instrumented_features = sys.argv[5]
warmup_iterations = int(sys.argv[6])
measured_iterations = int(sys.argv[7])
governor = sys.argv[8]
baseline_samples = json.loads(sys.argv[9])
instrumented_samples = json.loads(sys.argv[10])

if median_baseline_ns <= 0:
    raise SystemExit("median baseline must be nonzero")
overhead = (median_instrumented_ns - median_baseline_ns) / median_baseline_ns
refusal_reason = None
if overhead < 0:
    refusal_reason = (
        "negative overhead anomaly: instrumented median is below baseline; "
        "rerun under pinned measurement_env controls"
    )
report = {
    "schema": "s5_logging_overhead.v1",
    "workload_id": "tiny_preflight_shadow_compile",
    "warmup_iterations": warmup_iterations,
    "measured_iterations": measured_iterations,
    "median_baseline_ns": median_baseline_ns,
    "median_instrumented_ns": median_instrumented_ns,
    "overhead": overhead,
    "threshold": threshold,
    "pass": refusal_reason is None and overhead < threshold,
    "constitution_section": "II.1",
    "baseline_kind": {
        "command": f'cargo build --release -p gbf-train --bin s5_logging_overhead_workload --no-default-features --features "{baseline_features}"',
        "features": baseline_features,
        "binary_source": "target/release/s5_logging_overhead_workload",
        "logging_compiled_out": True,
    },
    "instrumented_kind": {
        "command": f'cargo build --release -p gbf-train --bin s5_logging_overhead_workload --no-default-features --features "{instrumented_features}"',
        "features": instrumented_features,
        "binary_source": "target/release/s5_logging_overhead_workload",
        "logging_compiled_out": False,
    },
    "measurement_env": {
        "cpu_governor": governor,
        "worker_threads": {
            "OMP_NUM_THREADS": "1",
            "RAYON_NUM_THREADS": "1",
            "BURN_NDARRAY_NUM_THREADS": "1",
        },
        "cache_state": "warm_after_warmup",
        "run_order": "interleaved_by_pair_alternating_first",
    },
    "samples": {
        "baseline_ns": baseline_samples,
        "instrumented_ns": instrumented_samples,
    },
}
if refusal_reason is not None:
    report["refusal_reason"] = refusal_reason
print(json.dumps(report, indent=2, sort_keys=True))
if refusal_reason is not None:
    print(f"D14 logging-overhead refused: {refusal_reason}", file=sys.stderr)
    raise SystemExit(2)
raise SystemExit(0 if report["pass"] else 1)
PY
}

run_self_test() {
    bash -n "$0"
    [[ "$BASELINE_FEATURES" == "s5-no-log,qat,burn-adapter" ]] || {
        echo "self-test failed: baseline feature string drifted" >&2
        exit 1
    }
    grep -F 'cargo build --release -p gbf-train --bin "$BIN_NAME"' "$0" >/dev/null || {
        echo "self-test failed: release build command missing" >&2
        exit 1
    }
    grep -F 'target/release/$BIN_NAME' "$0" >/dev/null || {
        echo "self-test failed: release binary source missing" >&2
        exit 1
    }

    if (
        unset OMP_NUM_THREADS RAYON_NUM_THREADS BURN_NDARRAY_NUM_THREADS S5_LOGGING_OVERHEAD_CPU_GOVERNOR
        export S5_LOGGING_OVERHEAD_SELF_TEST_NO_SYSFS=1
        validate_measurement_env
    ) >/tmp/s5-log-overhead-selftest.out 2>/tmp/s5-log-overhead-selftest.err; then
        echo "self-test failed: missing controls should refuse" >&2
        exit 1
    fi
    grep -F "OMP_NUM_THREADS must be 1" /tmp/s5-log-overhead-selftest.err >/dev/null

    (
        export OMP_NUM_THREADS=1
        export RAYON_NUM_THREADS=1
        export BURN_NDARRAY_NUM_THREADS=1
        export S5_LOGGING_OVERHEAD_CPU_GOVERNOR=performance
        export S5_LOGGING_OVERHEAD_SELF_TEST_NO_SYSFS=1
        validate_measurement_env
    )

    [[ "$(printf '{"logging_compiled_out":true}\n' | extract_workload_json_field logging_compiled_out)" == "true" ]] || {
        echo "self-test failed: bool JSON extraction drifted" >&2
        exit 1
    }
    validate_logging_compiled_out baseline true true
    if validate_logging_compiled_out instrumented true false >/tmp/s5-log-overhead-selftest.out 2>/tmp/s5-log-overhead-selftest.err; then
        echo "self-test failed: instrumented logging_compiled_out=true should refuse" >&2
        exit 1
    fi
    grep -F "instrumented workload reported logging_compiled_out=true" /tmp/s5-log-overhead-selftest.err >/dev/null

    echo "[S5 LOGGING OVERHEAD] self-test PASS"
}

case "$mode" in
    check_env)
        validate_measurement_env
        ;;
    self_test)
        run_self_test
        ;;
    run)
        run_gate
        ;;
    *)
        echo "internal error: unknown mode $mode" >&2
        exit 2
        ;;
esac
