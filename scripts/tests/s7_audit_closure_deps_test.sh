#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-audit-closure-deps-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT
issue="$tmp/issue.json"

python3 - "$issue" <<'PY'
from pathlib import Path
import json
import sys

issue = {
    "id": "bd-2v9r",
    "title": "Synthetic S7 closure",
    "status": "in_progress",
    "dependencies": [
        {
            "id": "bd-closed",
            "title": "Closed dependency",
            "status": "closed",
            "dep_type": "blocks",
        },
        {
            "id": "bd-related-open",
            "title": "Related open dependency",
            "status": "open",
            "dep_type": "related",
        },
    ],
    "comments": [],
}
Path(sys.argv[1]).write_text(json.dumps(issue, indent=2) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/audit-closure-deps.py --issue-file "$issue" \
  >/tmp/s7-audit-closure-deps-ok.out
rg -n "S7 closure dependency audit: ok" /tmp/s7-audit-closure-deps-ok.out >/dev/null

python3 - "$issue" <<'PY'
from pathlib import Path
import json
import sys

issue = {
    "id": "bd-2v9r",
    "title": "Synthetic S7 closure",
    "status": "in_progress",
    "dependencies": [
        {
            "id": "bd-open-blocker",
            "title": "Open blocker",
            "status": "open",
            "dep_type": "blocks",
        },
        {
            "id": "bd-open-parent",
            "title": "Open parent",
            "status": "open",
            "dep_type": "parent-child",
        },
    ],
    "comments": [],
}
Path(sys.argv[1]).write_text(json.dumps(issue, indent=2) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/audit-closure-deps.py --issue-file "$issue" \
  >/tmp/s7-audit-closure-deps-bad.out 2>&1; then
  echo "expected unresolved closure dependencies to fail audit" >&2
  exit 1
fi
rg -n "bd-2v9r: unresolved dependency bd-open-blocker status=open type=blocks" \
  /tmp/s7-audit-closure-deps-bad.out >/dev/null
rg -n "bd-2v9r: unresolved dependency bd-open-parent status=open type=parent-child" \
  /tmp/s7-audit-closure-deps-bad.out >/dev/null

scripts/review/f-s7/audit-closure-deps.py --issue-file "$issue" --json \
  >/tmp/s7-audit-closure-deps-json.out || true
rg -n '"status": "needs_changes"' /tmp/s7-audit-closure-deps-json.out >/dev/null

python3 - "$issue" <<'PY'
from pathlib import Path
import json
import sys

issue = {
    "id": "bd-2v9r",
    "title": "Synthetic S7 closure",
    "status": "in_progress",
    "dependencies": [
        {
            "id": "bd-open-blocker",
            "title": "Open blocker",
            "status": "open",
            "dep_type": "blocks",
        },
    ],
    "comments": [
        {
            "text": (
                "S7 closure dependency disposition: bd-open-blocker non-blocking. "
                "Manager reviewed the stale edge and moved the remaining work."
            ),
        }
    ],
}
Path(sys.argv[1]).write_text(json.dumps(issue, indent=2) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/audit-closure-deps.py --issue-file "$issue" \
  >/tmp/s7-audit-closure-deps-disposition-ok.out
rg -n "S7 closure dependency audit: ok" \
  /tmp/s7-audit-closure-deps-disposition-ok.out >/dev/null

echo "s7_audit_closure_deps_test: ok"
