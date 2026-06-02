#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/s5_logging_overhead_check.sh"

bash -n "$SCRIPT"
"$SCRIPT" --self-test >/dev/null

if (
    unset OMP_NUM_THREADS RAYON_NUM_THREADS BURN_NDARRAY_NUM_THREADS S5_LOGGING_OVERHEAD_CPU_GOVERNOR
    export S5_LOGGING_OVERHEAD_SELF_TEST_NO_SYSFS=1
    "$SCRIPT" --check-env-only
) >/tmp/s5-logging-overhead-check-test.out 2>/tmp/s5-logging-overhead-check-test.err; then
    echo "s5_logging_overhead_check.sh should refuse when measurement_env controls are absent" >&2
    exit 1
fi
grep -F "D14 logging-overhead refused" /tmp/s5-logging-overhead-check-test.err >/dev/null

(
    export OMP_NUM_THREADS=1
    export RAYON_NUM_THREADS=1
    export BURN_NDARRAY_NUM_THREADS=1
    export S5_LOGGING_OVERHEAD_CPU_GOVERNOR=performance
    export S5_LOGGING_OVERHEAD_SELF_TEST_NO_SYSFS=1
    "$SCRIPT" --check-env-only
)

grep -F 'BASELINE_FEATURES="s5-no-log,qat,burn-adapter"' "$SCRIPT" >/dev/null
grep -F 'cargo build --release -p gbf-train --bin "$BIN_NAME"' "$SCRIPT" >/dev/null
grep -F 'target/release/$BIN_NAME' "$SCRIPT" >/dev/null
grep -F 'logging_compiled_out' "$SCRIPT" >/dev/null
if grep -F 'max(0.0' "$SCRIPT" >/dev/null; then
    echo "s5_logging_overhead_check.sh must not clamp signed overhead" >&2
    exit 1
fi

echo "[S5 LOGGING OVERHEAD TEST] script checks passed"
