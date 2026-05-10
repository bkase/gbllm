#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash -n "$ROOT/scripts/s1_canonical_o_metric_4_check.sh"
python3 -m py_compile "$ROOT/scripts/download_tinystories.py"
"$ROOT/scripts/s1_canonical_o_metric_4_check.sh" --self-test

echo "[O-METRIC-4 TEST] canonical scheduled check harness passed"
