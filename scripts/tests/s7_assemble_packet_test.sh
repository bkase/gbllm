#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-assemble-packet-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

manifest="$tmp/bundle/manifest.json"
mkdir -p "$(dirname "$manifest")"

scripts/review/f-s7/assemble-packet.py --write-template "$manifest" >"$tmp/template.out"
rg -n "wrote manifest template" "$tmp/template.out" >/dev/null
rg -n '"schema": "s7_production_bundle_manifest.v1"' "$manifest" >/dev/null

scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates \
  --dry-run >"$tmp/dry-run.out"

rg -c " materialize-run " "$tmp/dry-run.out" | rg '^10$' >/dev/null
rg -c " --kind switch-stats " "$tmp/dry-run.out" | rg '^5$' >/dev/null
rg -n "derive-summaries" "$tmp/dry-run.out" >/dev/null
rg -n "derive-comparison" "$tmp/dry-run.out" >/dev/null
rg -n "derive-frontier" "$tmp/dry-run.out" >/dev/null
rg -n "emit-report" "$tmp/dry-run.out" >/dev/null
rg -n "verify-packet\\.sh --skip-gates" "$tmp/dry-run.out" >/dev/null
rg -n -- "--topology MoeTinyDenseMatched" "$tmp/dry-run.out" >/dev/null
rg -n -- "$tmp/bundle/runs/MoeTiny/seed-0/run-log\\.json" "$tmp/dry-run.out" >/dev/null
rg -n "S7 packet assembly: dry-run ok" "$tmp/dry-run.out" >/dev/null

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates \
  --dry-run \
  --check-inputs >"$tmp/check-inputs.out" 2>&1; then
  echo "expected check-inputs dry-run to fail for missing bundle files" >&2
  exit 1
fi
rg -n "missing input file: .*/bundle/runs/MoeTiny/seed-0/run-log\\.json" "$tmp/check-inputs.out" >/dev/null
if rg -n " materialize-run " "$tmp/check-inputs.out" >/dev/null; then
  echo "check-inputs should preflight before printing executable commands" >&2
  exit 1
fi

python3 - "$manifest" "$tmp/bad-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["comparison"]["moe_topology_hash"] = "not-a-hash"
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/bad-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/bad.out" 2>&1; then
  echo "expected invalid comparison hash to fail" >&2
  exit 1
fi
rg -n "comparison\\.moe_topology_hash must be a sha256 hash" "$tmp/bad.out" >/dev/null

python3 - "$manifest" "$tmp/unknown-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["runs"]["MoeTiny"]["0"]["runlog"] = payload["runs"]["MoeTiny"]["0"].pop("run_log")
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/unknown-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/unknown.out" 2>&1; then
  echo "expected unknown manifest field to fail" >&2
  exit 1
fi
rg -n "runs\\.MoeTiny\\.0 has unknown field\\(s\\): runlog" "$tmp/unknown.out" >/dev/null

if scripts/review/f-s7/assemble-packet.py --root "$ROOT" --dry-run >"$tmp/missing.out" 2>&1; then
  echo "expected missing manifest argument to fail" >&2
  exit 1
fi
rg -n -- "--manifest is required unless --write-template is used" "$tmp/missing.out" >/dev/null

echo "s7_assemble_packet_test: ok"
