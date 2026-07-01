#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-audit-production-runner-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

closure="$tmp/closure.json"
runner_open="$tmp/runner-open.json"
runner_closed="$tmp/runner-closed.json"
runner_weak="$tmp/runner-weak.json"

cat >"$closure" <<'JSON'
{
  "id": "bd-2v9r",
  "title": "Slice S7 closure: MoE beats dense at matched bytes",
  "dependencies": [
    {
      "id": "bd-3e10j",
      "title": "F-S7 production training runner",
      "status": "in_progress",
      "dep_type": "blocks"
    }
  ]
}
JSON

cat >"$runner_open" <<'JSON'
{
  "id": "bd-3e10j",
  "title": "F-S7 production training runner: real Gutenberg MoE+dense 5-seed bundle producer",
  "description": "Implement the real F-S7 production runner for s7_production_bundle_manifest.v1. It trains MoeTiny and MoeTinyDenseMatched on Gutenberg with optimizer/model-state updates and emits s7_run_log.v1, s7_grad_log.v1, s7_router_step_telemetry.v1, validate-artifacts.py-compatible outputs, and production_closure_retrain_score sweep evidence.",
  "acceptance_criteria": "s7_production_bundle_manifest.v1",
  "status": "in_progress",
  "comments": []
}
JSON

cat >"$runner_closed" <<'JSON'
{
  "id": "bd-3e10j",
  "title": "F-S7 production training runner: real Gutenberg MoE+dense 5-seed bundle producer",
  "description": "Implement the real F-S7 production runner for s7_production_bundle_manifest.v1. It trains MoeTiny and MoeTinyDenseMatched on Gutenberg with optimizer/model-state updates and emits s7_run_log.v1, s7_grad_log.v1, s7_router_step_telemetry.v1, validate-artifacts.py-compatible outputs, and production_closure_retrain_score sweep evidence.",
  "acceptance_criteria": "s7_production_bundle_manifest.v1",
  "status": "closed",
  "comments": []
}
JSON

cat >"$runner_weak" <<'JSON'
{
  "id": "bd-3e10j",
  "title": "F-S7 production training runner",
  "description": "placeholder",
  "status": "closed",
  "comments": []
}
JSON

if scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$closure" \
  --runner-issue-file "$runner_open" >"$tmp/open.out"; then
  echo "expected unresolved bd-3e10j to fail" >&2
  exit 1
fi
rg -n "do not consume fixture/smoke/replay artifacts as production" "$tmp/open.out" >/dev/null

mkdir -p "$tmp/experiments/S7/runs/MoeTiny/seed-0"
printf '{}\n' >"$tmp/experiments/S7/runs/MoeTiny/seed-0/run-log.json"
if scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$closure" \
  --runner-issue-file "$runner_open" >"$tmp/artifacts.out"; then
  echo "expected unresolved runner plus production-looking artifacts to fail" >&2
  exit 1
fi
rg -n "production-looking S7 artifacts exist while runner owner is unresolved" "$tmp/artifacts.out" >/dev/null
rm -rf "$tmp/experiments"

missing_dep="$tmp/closure-missing-dep.json"
cat >"$missing_dep" <<'JSON'
{"id":"bd-2v9r","title":"Slice S7 closure","dependencies":[]}
JSON
if scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$missing_dep" \
  --runner-issue-file "$runner_closed" >"$tmp/missing-dep.out"; then
  echo "expected missing bd-3e10j dependency to fail" >&2
  exit 1
fi
rg -n "missing blocking dependency on bd-3e10j" "$tmp/missing-dep.out" >/dev/null

if scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$closure" \
  --runner-issue-file "$runner_weak" >"$tmp/weak.out"; then
  echo "expected weak runner contract to fail" >&2
  exit 1
fi
rg -n "production-runner contract missing required phrase" "$tmp/weak.out" >/dev/null

scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$closure" \
  --runner-issue-file "$runner_closed" >"$tmp/ok.out"
rg -n "S7 production runner audit: ok" "$tmp/ok.out" >/dev/null

scripts/review/f-s7/audit-production-runner.py \
  --root "$tmp" \
  --skip-code-surface \
  --closure-issue-file "$closure" \
  --runner-issue-file "$runner_closed" \
  --json >"$tmp/ok.json"
rg -n '"runner_resolved": true' "$tmp/ok.json" >/dev/null

echo "s7_audit_production_runner_test: ok"
