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
rg -n '"decision": "ProceedToS8"' "$manifest" >/dev/null

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
rg -n -- "--decision ProceedToS8" "$tmp/dry-run.out" >/dev/null
rg -n "verify-packet\\.sh --skip-gates" "$tmp/dry-run.out" >/dev/null
rg -n -- "--topology MoeTinyDenseMatched" "$tmp/dry-run.out" >/dev/null
rg -n -- "$tmp/bundle/runs/MoeTiny/seed-0/run-log\\.json" "$tmp/dry-run.out" >/dev/null
rg -n "S7 packet assembly: dry-run ok" "$tmp/dry-run.out" >/dev/null

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates >"$tmp/no-run-reviews.out" 2>&1; then
  echo "expected non-dry-run assembly without --run-reviews to fail" >&2
  exit 1
fi
rg -n "non-dry-run production assembly requires --run-reviews" "$tmp/no-run-reviews.out" >/dev/null
if rg -n "missing input file:" "$tmp/no-run-reviews.out" >/dev/null; then
  echo "non-dry-run assembly must require reviews before input preflight" >&2
  exit 1
fi

scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates \
  --run-reviews \
  --review-cwd "$ROOT" \
  --acpx custom-acpx \
  --review-timeout 77 \
  --gemini-agent "custom-gemini --acp" \
  --claude-agent "custom-claude --acp" \
  --dry-run >"$tmp/dry-run-reviews.out"

rg -n "run-acpx-reviews\\.py" "$tmp/dry-run-reviews.out" >/dev/null
rg -n -- "--acpx custom-acpx" "$tmp/dry-run-reviews.out" >/dev/null
rg -n -- "--timeout 77" "$tmp/dry-run-reviews.out" >/dev/null
rg -n -- "--reviewer all" "$tmp/dry-run-reviews.out" >/dev/null
rg -n -- "--gemini-agent 'custom-gemini --acp'" "$tmp/dry-run-reviews.out" >/dev/null
rg -n -- "--claude-agent 'custom-claude --acp'" "$tmp/dry-run-reviews.out" >/dev/null
python3 - "$tmp/dry-run-reviews.out" <<'PY'
from pathlib import Path
import sys

text = Path(sys.argv[1]).read_text(encoding="utf-8")
report_index = text.index(" emit-report ")
review_index = text.index("run-acpx-reviews.py")
verify_index = text.index("verify-packet.sh --skip-gates")
if not (report_index < review_index < verify_index):
    raise SystemExit("run-acpx-reviews.py must run after emit-report and before verify-packet")
PY

scripts/review/f-s7/assemble-packet.py \
  --manifest "$manifest" \
  --root "$ROOT" \
  --cargo cargo \
  --verify-mode skip-gates \
  --run-reviews \
  --dry-run >"$tmp/dry-run-reviews-default-cwd.out"

rg -n "run-acpx-reviews\\.py" "$tmp/dry-run-reviews-default-cwd.out" >/dev/null
rg -n -- "--review-cwd $ROOT" "$tmp/dry-run-reviews-default-cwd.out" >/dev/null
if rg -n -- "--review-cwd /Users/bkase/Documents/gbllm" "$tmp/dry-run-reviews-default-cwd.out" >/dev/null; then
  echo "assemble-packet must not default ACPX reviews to a sibling checkout" >&2
  exit 1
fi

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

python3 - "$manifest" "$tmp/bad-decision-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["report"]["decision"] = "ProceedToS8DenseOnly"
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/bad-decision-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/bad-decision.out" 2>&1; then
  echo "expected report decision mismatch to fail" >&2
  exit 1
fi
rg -n "report\\.decision must be ProceedToS8 when report\\.s7_outcome is PassClean" "$tmp/bad-decision.out" >/dev/null
if rg -n " materialize-run " "$tmp/bad-decision.out" >/dev/null; then
  echo "invalid report decision should fail before printing executable commands" >&2
  exit 1
fi

python3 - "$manifest" "$tmp/missing-decision-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["report"].pop("decision")
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/missing-decision-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/missing-decision.out" 2>&1; then
  echo "expected missing report decision to fail" >&2
  exit 1
fi
rg -n "report\\.decision must be a non-empty string" "$tmp/missing-decision.out" >/dev/null

python3 - "$manifest" "$tmp/bad-rfc-revision-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["report"]["rfc_revision"] = "not-a-revision"
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/bad-rfc-revision-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/bad-rfc-revision.out" 2>&1; then
  echo "expected invalid report rfc_revision to fail" >&2
  exit 1
fi
rg -n "report\\.rfc_revision must be a 40-hex git commit id or sha256 hash" "$tmp/bad-rfc-revision.out" >/dev/null

python3 - "$manifest" "$tmp/missing-rfc-revision-manifest.json" <<'PY'
from pathlib import Path
import json
import sys

payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
payload["report"].pop("rfc_revision")
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/assemble-packet.py \
  --manifest "$tmp/missing-rfc-revision-manifest.json" \
  --root "$ROOT" \
  --dry-run >"$tmp/missing-rfc-revision.out" 2>&1; then
  echo "expected missing report rfc_revision to fail" >&2
  exit 1
fi
rg -n "report\\.rfc_revision must be a non-empty string" "$tmp/missing-rfc-revision.out" >/dev/null

if scripts/review/f-s7/assemble-packet.py --root "$ROOT" --dry-run >"$tmp/missing.out" 2>&1; then
  echo "expected missing manifest argument to fail" >&2
  exit 1
fi
rg -n -- "--manifest is required unless --write-template is used" "$tmp/missing.out" >/dev/null

echo "s7_assemble_packet_test: ok"
