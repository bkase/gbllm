#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

if rg -n '\.tensors\b|QuantGraph::[^[:space:]]*tensors' gbf-codegen/src/s5; then
  echo "error: Stage 5 RangePlan must not read QuantGraph tensor internals" >&2
  exit 1
fi

# `rg -U` lets the struct-literal guard span lines, so a multiline
# `QuantGraph { ... tensors: ... }` fixture cannot evade the Stage 5 boundary.
if rg -n -U 'QuantGraph[[:space:]]*\{([^}]|\n)*\btensors\b' gbf-codegen/src/s5; then
  echo "error: Stage 5 RangePlan must not read QuantGraph tensor internals" >&2
  exit 1
fi
