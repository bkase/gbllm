#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-validate-reviews-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
review_dir="$tmp/docs/review/f-s7/reviews"
mkdir -p "$review_dir"
expected_head=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

python3 - "$review_dir" "$expected_head" <<'PY'
from pathlib import Path
import json
import sys

review_dir = Path(sys.argv[1])
head = sys.argv[2]

payloads = {
    "gemini": ["P3", "P4", "P5", "P6", "P7", "P8"],
    "claude": ["P3", "P5", "P6", "P8"],
}
for reviewer, personas in payloads.items():
    payload = {
        "schema": "s7_acpx_review.v1",
        "bead": "bd-2v9r",
        "reviewer": reviewer,
        "transport": "acpx",
        "verdict": "PASS",
        "personas": personas,
        "command": f"acpx {reviewer} exec review",
        "reviewed_head": head,
        "summary": f"{reviewer} passed synthetic review evidence.",
        "findings": [],
    }
    (review_dir / f"bd-2v9r-{reviewer}.json").write_text(
        json.dumps(payload, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
PY

scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-ok.out

python3 - "$review_dir/bd-2v9r-gemini.json" <<'PY'
from pathlib import Path
import json
import sys

gemini = Path(sys.argv[1])
payload = json.loads(gemini.read_text(encoding="utf-8"))
payload["command"] = "manual review mentioning acpx"
gemini.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected non-acpx command prefix to fail" >&2
  exit 1
fi
rg -n "command must record an ACPX invocation prefix" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-gemini.json" <<'PY'
from pathlib import Path
import json
import sys

gemini = Path(sys.argv[1])
payload = json.loads(gemini.read_text(encoding="utf-8"))
payload["command"] = "acpx gemini exec review"
gemini.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected reviewed_head mismatch to fail" >&2
  exit 1
fi
rg -n "reviewed_head must match expected_head" /tmp/s7-validate-reviews-bad.out >/dev/null

ancestor_repo="$tmp/ancestor-repo"
ancestor_review_dir="$ancestor_repo/docs/review/f-s7/reviews"
mkdir -p "$ancestor_review_dir" "$ancestor_repo/.beads"
git -C "$ancestor_repo" init -q
git -C "$ancestor_repo" config user.name "S7 Review Test"
git -C "$ancestor_repo" config user.email "s7-review-test@example.invalid"
printf 'artifact\n' >"$ancestor_repo/artifact.txt"
git -C "$ancestor_repo" add artifact.txt
git -C "$ancestor_repo" commit -qm base
ancestor_head="$(git -C "$ancestor_repo" rev-parse HEAD)"

python3 - "$ancestor_review_dir" "$ancestor_head" <<'PY'
from pathlib import Path
import json
import sys

review_dir = Path(sys.argv[1])
head = sys.argv[2]

payloads = {
    "gemini": ["P3", "P4", "P5", "P6", "P7", "P8"],
    "claude": ["P3", "P5", "P6", "P8"],
}
for reviewer, personas in payloads.items():
    payload = {
        "schema": "s7_acpx_review.v1",
        "bead": "bd-2v9r",
        "reviewer": reviewer,
        "transport": "acpx",
        "verdict": "PASS",
        "personas": personas,
        "command": f"acpx {reviewer} exec review",
        "reviewed_head": head,
        "summary": f"{reviewer} passed ancestor review evidence.",
        "findings": [],
    }
    (review_dir / f"bd-2v9r-{reviewer}.json").write_text(
        json.dumps(payload, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
PY

git -C "$ancestor_repo" add docs/review/f-s7/reviews
git -C "$ancestor_repo" commit -qm reviews
ancestor_current="$(git -C "$ancestor_repo" rev-parse HEAD)"
scripts/review/f-s7/validate-reviews.py \
  --root "$ancestor_repo" \
  --allow-reviewed-head-ancestor-of "$ancestor_current" \
  --require-reviewed-diff-admin-only >/tmp/s7-validate-reviews-ancestor-ok.out

printf '{}\n' >"$ancestor_repo/.beads/issues.jsonl"
git -C "$ancestor_repo" add .beads/issues.jsonl
git -C "$ancestor_repo" commit -qm bead-admin
ancestor_admin_head="$(git -C "$ancestor_repo" rev-parse HEAD)"
scripts/review/f-s7/validate-reviews.py \
  --root "$ancestor_repo" \
  --allow-reviewed-head-ancestor-of "$ancestor_admin_head" \
  --require-reviewed-diff-admin-only >/tmp/s7-validate-reviews-ancestor-admin-ok.out

printf 'changed after review\n' >"$ancestor_repo/src.txt"
git -C "$ancestor_repo" add src.txt
git -C "$ancestor_repo" commit -qm stale-production-change
ancestor_stale_head="$(git -C "$ancestor_repo" rev-parse HEAD)"
if scripts/review/f-s7/validate-reviews.py \
  --root "$ancestor_repo" \
  --allow-reviewed-head-ancestor-of "$ancestor_stale_head" \
  --require-reviewed-diff-admin-only >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected stale post-review production change to fail" >&2
  exit 1
fi
rg -n "reviewed_head is stale: commits after it changed non-review files" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-gemini.json" "$review_dir/bd-2v9r-claude.json" <<'PY'
from pathlib import Path
import json
import sys

gemini = Path(sys.argv[1])
payload = json.loads(gemini.read_text(encoding="utf-8"))
payload["personas"] = ["P3", "P6", "P7", "P8"]
gemini.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")

claude = Path(sys.argv[2])
payload = json.loads(claude.read_text(encoding="utf-8"))
payload["verdict"] = "CONCERNS"
claude.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected invalid review evidence to fail" >&2
  exit 1
fi
rg -n "missing required personas for gemini" /tmp/s7-validate-reviews-bad.out >/dev/null
rg -n "missing always-on persona P5" /tmp/s7-validate-reviews-bad.out >/dev/null
if rg -n "missing required personas for gemini: .*P5" /tmp/s7-validate-reviews-bad.out >/dev/null; then
  echo "always-on P5 should not be duplicated in reviewer-specific missing-persona diagnostics" >&2
  exit 1
fi
rg -n "verdict must be 'PASS'" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-claude.json" <<'PY'
from pathlib import Path
import json
import sys

claude = Path(sys.argv[1])
payload = json.loads(claude.read_text(encoding="utf-8"))
payload["verdict"] = "PASS"
payload["findings"] = [{"severity": "major", "status": "open", "body": "synthetic blocker"}]
claude.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected unresolved blocking finding to fail" >&2
  exit 1
fi
rg -n "PASS review has unresolved blocking finding" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-claude.json" <<'PY'
from pathlib import Path
import json
import sys

claude = Path(sys.argv[1])
payload = json.loads(claude.read_text(encoding="utf-8"))
payload["findings"] = [{"status": "non_blocking", "body": "missing severity"}]
claude.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected missing severity finding to fail" >&2
  exit 1
fi
rg -n "severity must be one of" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-claude.json" <<'PY'
from pathlib import Path
import json
import sys

claude = Path(sys.argv[1])
payload = json.loads(claude.read_text(encoding="utf-8"))
payload["findings"] = [{"severity": "medium", "status": "triaged", "body": "bad status"}]
claude.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected invalid status finding to fail" >&2
  exit 1
fi
rg -n "status must be one of" /tmp/s7-validate-reviews-bad.out >/dev/null

python3 - "$review_dir/bd-2v9r-claude.json" <<'PY'
from pathlib import Path
import json
import sys

claude = Path(sys.argv[1])
payload = json.loads(claude.read_text(encoding="utf-8"))
payload["findings"] = ["not an object"]
claude.write_text(json.dumps(payload, sort_keys=True, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-reviews.py --root "$tmp" --expected-head "$expected_head" >/tmp/s7-validate-reviews-bad.out 2>&1; then
  echo "expected non-object finding to fail" >&2
  exit 1
fi
rg -n "finding findings\\[0\\] must be an object" /tmp/s7-validate-reviews-bad.out >/dev/null

echo "s7_validate_reviews_test: ok"
