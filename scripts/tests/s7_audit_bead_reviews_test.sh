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
        "title": "Synthetic S7 bead with structured file evidence",
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

scripts/review/f-s7/audit-bead-reviews.py \
  --issues-file "$issues" \
  --evidence-dir "$evidence_dir" \
  --expected-head aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  >/tmp/s7-audit-bead-reviews-head-ok.out
rg -n "S7 bead review coverage: ok" /tmp/s7-audit-bead-reviews-head-ok.out >/dev/null

if scripts/review/f-s7/audit-bead-reviews.py \
  --issues-file "$issues" \
  --evidence-dir "$evidence_dir" \
  --expected-head bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  >/tmp/s7-audit-bead-reviews-head-bad.out 2>&1; then
  echo "expected reviewed_head mismatch to fail audit" >&2
  exit 1
fi
rg -n "reviewed_head must match expected_head" /tmp/s7-audit-bead-reviews-head-bad.out >/dev/null

review_repo="$tmp/review-repo"
review_issues="$tmp/review-issues.json"
mkdir -p "$review_repo"
git -C "$review_repo" init -q
git -C "$review_repo" config user.email s7-test@example.com
git -C "$review_repo" config user.name "S7 Test"
printf 'initial\n' >"$review_repo/README.md"
git -C "$review_repo" add README.md
git -C "$review_repo" commit -q -m init
review_base="$(git -C "$review_repo" rev-parse HEAD)"
mkdir -p "$review_repo/docs/review/f-s7/bead-reviews"
python3 - "$review_issues" "$review_repo/docs/review/f-s7/bead-reviews/bd-file-gemini.json" "$review_base" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-file",
        "title": "Synthetic S7 bead with review-admin-only post-review diff",
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
    "reviewed_head": sys.argv[3],
    "summary": "Structured Gemini bead review passed before evidence was committed.",
    "findings": [],
}
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY
git -C "$review_repo" add docs/review/f-s7/bead-reviews/bd-file-gemini.json
git -C "$review_repo" commit -q -m "add review evidence"
review_admin_head="$(git -C "$review_repo" rev-parse HEAD)"

scripts/review/f-s7/audit-bead-reviews.py \
  --root "$review_repo" \
  --issues-file "$review_issues" \
  --evidence-dir "$review_repo/docs/review/f-s7/bead-reviews" \
  --allow-reviewed-head-ancestor-of "$review_admin_head" \
  --require-reviewed-diff-admin-only \
  >/tmp/s7-audit-bead-reviews-ancestor-ok.out
rg -n "S7 bead review coverage: ok" /tmp/s7-audit-bead-reviews-ancestor-ok.out >/dev/null

mkdir -p "$review_repo/src"
printf 'pub fn changed_after_review() {}\n' >"$review_repo/src/lib.rs"
git -C "$review_repo" add src/lib.rs
git -C "$review_repo" commit -q -m "change code after review"
review_stale_head="$(git -C "$review_repo" rev-parse HEAD)"
if scripts/review/f-s7/audit-bead-reviews.py \
  --root "$review_repo" \
  --issues-file "$review_issues" \
  --evidence-dir "$review_repo/docs/review/f-s7/bead-reviews" \
  --allow-reviewed-head-ancestor-of "$review_stale_head" \
  --require-reviewed-diff-admin-only \
  >/tmp/s7-audit-bead-reviews-ancestor-bad.out 2>&1; then
  echo "expected non-review post-review diff to fail audit" >&2
  exit 1
fi
rg -n "reviewed_head is stale: commits after it changed non-review files" \
  /tmp/s7-audit-bead-reviews-ancestor-bad.out >/dev/null

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

python3 - "$issues" "$evidence_dir/bd-file-gemini.json" <<'PY'
from pathlib import Path
import json
import sys

issues = [
    {
        "id": "bd-file",
        "title": "Synthetic S7 bead with weak structured evidence",
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
    "personas": ["P1", "P2"],
    "command": "acpx --agent 'gemini --acp' exec review",
    "reviewed_head": "a" * 40,
    "summary": "Structured Gemini bead review omitted always-on personas.",
    "findings": [],
}
Path(sys.argv[2]).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/audit-bead-reviews.py \
  --issues-file "$issues" \
  --evidence-dir "$evidence_dir" \
  >/tmp/s7-audit-bead-reviews-evidence-weak.out 2>&1; then
  echo "expected missing always-on personas to fail audit" >&2
  exit 1
fi
rg -n "missing always-on persona" /tmp/s7-audit-bead-reviews-evidence-weak.out >/dev/null

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
