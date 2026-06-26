#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-audit-bead-reviews-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
issues="$tmp/issues.json"
evidence_dir="$tmp/evidence"

python3 - "$issues" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-pass",
        "title": "Synthetic S7 closed bead",
        "status": "closed",
        "close_reason": "Gemini ACPX review: PASS\nClaude ACPX review: PASS",
        "comments": [],
    },
    {
        "id": "bd-manager",
        "title": "Synthetic S7 dispositioned bead",
        "status": "closed",
        "close_reason": "Gemini ACPX review: NEEDS_CHANGES\nClaude ACPX review: PASS",
        "comments": [
            {
                "text": "Manager disposition: follow-up absorbed the Gemini finding. No additional ACPX review required.",
            }
        ],
    },
    {
        "id": "bd-non-blocking",
        "title": "Synthetic S7 non-blocking concern bead",
        "status": "closed",
        "close_reason": (
            "Gemini ACPX review: PASS\n"
            "Claude CONCERNS were closure-hygiene only, not code blockers."
        ),
        "comments": [],
    },
    {
        "id": "bd-tombstone",
        "title": "Synthetic S7 consolidated bead",
        "status": "tombstone",
        "close_reason": "",
        "comments": [],
    },
]
Path(sys.argv[1]).write_text(json.dumps(issues, indent=2) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/audit-bead-reviews.py --issues-file "$issues" \
  >/tmp/s7-audit-bead-reviews-ok.out
rg -n "S7 bead review coverage: ok" /tmp/s7-audit-bead-reviews-ok.out >/dev/null

mkdir -p "$evidence_dir"
python3 - "$issues" "$evidence_dir/bd-file-gemini.json" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-file",
        "title": "Synthetic S7 bead with structured Gemini evidence",
        "status": "closed",
        "close_reason": "Claude ACPX review: PASS",
        "comments": [],
    }
]
Path(sys.argv[1]).write_text(json.dumps(issues, indent=2) + "\n", encoding="utf-8")
payload = {
    "schema": "s7_bead_acpx_review.v1",
    "bead": "bd-file",
    "reviewer": "gemini",
    "transport": "acpx",
    "verdict": "PASS",
    "personas": ["P1", "P2", "P4", "P5", "P6", "P8"],
    "command": "acpx --agent 'gemini --acp' exec review",
    "reviewed_head": "a" * 40,
    "summary": "Structured Gemini bead review passed.",
    "findings": [],
}
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/audit-bead-reviews.py \
  --issues-file "$issues" \
  --evidence-dir "$evidence_dir" \
  >/tmp/s7-audit-bead-reviews-evidence-ok.out
rg -n "S7 bead review coverage: ok" /tmp/s7-audit-bead-reviews-evidence-ok.out >/dev/null

python3 - "$issues" "$evidence_dir/bd-file-gemini.json" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-file",
        "title": "Synthetic S7 bead with bad structured evidence",
        "status": "closed",
        "close_reason": "Claude ACPX review: PASS",
        "comments": [],
    }
]
Path(sys.argv[1]).write_text(json.dumps(issues, indent=2) + "\n", encoding="utf-8")
payload = {
    "schema": "s7_bead_acpx_review.v1",
    "bead": "bd-file",
    "reviewer": "gemini",
    "transport": "acpx",
    "verdict": "NEEDS_CHANGES",
    "personas": ["P1", "P2", "P4", "P5", "P6", "P8"],
    "command": "acpx --agent 'gemini --acp' exec review",
    "reviewed_head": "a" * 40,
    "summary": "Structured Gemini bead review did not pass.",
    "findings": [],
}
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/audit-bead-reviews.py \
  --issues-file "$issues" \
  --evidence-dir "$evidence_dir" \
  >/tmp/s7-audit-bead-reviews-evidence-bad.out 2>&1; then
  echo "expected non-PASS structured evidence to fail audit" >&2
  exit 1
fi
rg -n "verdict must be 'PASS'" /tmp/s7-audit-bead-reviews-evidence-bad.out >/dev/null

python3 - "$issues" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-failed-gemini",
        "title": "Synthetic S7 Gemini route failure",
        "status": "closed",
        "close_reason": (
            "Claude ACPX review: PASS\n"
            "Gemini ACPX review: attempted, but failed before review. No Gemini PASS is claimed."
        ),
        "comments": [],
    }
]
Path(sys.argv[1]).write_text(json.dumps(issues, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/audit-bead-reviews.py --issues-file "$issues" \
  >/tmp/s7-audit-bead-reviews-bad.out 2>&1; then
  echo "expected failed Gemini route to fail audit" >&2
  exit 1
fi
rg -n "bd-failed-gemini: missing gemini review evidence" \
  /tmp/s7-audit-bead-reviews-bad.out >/dev/null

python3 - "$issues" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-open",
        "title": "Synthetic S7 open bead",
        "status": "in_progress",
        "close_reason": "Gemini ACPX review: PASS\nClaude ACPX review: PASS",
        "comments": [],
    }
]
Path(sys.argv[1]).write_text(json.dumps(issues, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/audit-bead-reviews.py --issues-file "$issues" \
  >/tmp/s7-audit-bead-reviews-bad.out 2>&1; then
  echo "expected open bead to fail audit" >&2
  exit 1
fi
rg -n "bd-open: status is in_progress" /tmp/s7-audit-bead-reviews-bad.out >/dev/null

scripts/review/f-s7/audit-bead-reviews.py --issues-file "$issues" --json \
  >/tmp/s7-audit-bead-reviews-json.out || true
rg -n '"status": "needs_changes"' /tmp/s7-audit-bead-reviews-json.out >/dev/null

echo "s7_audit_bead_reviews_test: ok"
