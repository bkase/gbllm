#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

failures=0

fail() {
  echo "error: $*" >&2
  failures=1
}

rust_code_match() {
  local pattern="$1"
  shift

  if (($# == 0)); then
    return 1
  fi

  rg -n --glob '*.rs' '^[[:space:]]*[^/[:space:]].*('"$pattern"')' "$@"
}

stage_dirs=(
  "gbf-codegen/src/s4"
  "gbf-codegen/src/s5"
)

existing_stage_dirs=()
for dir in "${stage_dirs[@]}"; do
  if [[ -d "$dir" ]]; then
    existing_stage_dirs+=("$dir")
  fi
done

if ((${#existing_stage_dirs[@]} > 0)); then
  if rust_code_match '\bProbeBudgetClass\b|gbf_abi::trace::ProbeBudgetClass' "${existing_stage_dirs[@]}"; then
    fail "F-B6/F-B7 policy stages must use gbf_policy::probe::ProbeImportanceClass, not the ABI ProbeBudgetClass"
  fi
else
  echo "info: gbf-codegen/src/s4 and src/s5 are absent; policy/runtime vocabulary guards still ran" >&2
fi

if rust_code_match '\bProbeImportanceClass\b' gbf-runtime; then
  fail "gbf-runtime must stay on gbf_abi::trace budget/window types and must not use policy ProbeImportanceClass"
fi

if rust_code_match 'use[[:space:]]+gbf_abi::trace::.*\bProbeBudgetClass\b|gbf_abi::trace::ProbeBudgetClass' gbf-policy/src; then
  fail "gbf-policy must not import or use gbf_abi::trace::ProbeBudgetClass; only comments may mention the collision"
fi

required_patterns=(
  'pub enum ProbeImportanceClass'
  'impl<'\''de> Deserialize<'\''de> for ProbeImportanceClass'
  'fn probe_importance_class_public_json_shapes_are_pinned'
  'fn probe_importance_class_serde_rejects_unknown'
  'impl From<TraceProbeId> for gbf_abi::trace::TraceProbeId'
  'impl From<gbf_abi::trace::TraceProbeId> for TraceProbeId'
  'fn trace_probe_id_conversion_preserves_edge_values'
  'fn trace_probe_id_json_shape_is_u16'
)

for pattern in "${required_patterns[@]}"; do
  if ! rg -n "$pattern" gbf-policy/src >/dev/null; then
    fail "missing required probe vocabulary guard/test pattern: $pattern"
  fi
done

if ((failures > 0)); then
  exit 1
fi
