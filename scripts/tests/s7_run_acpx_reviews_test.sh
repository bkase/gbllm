#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-run-acpx-reviews-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

make_repo() {
  local repo="$1"
  mkdir -p "$repo"
  git -C "$repo" init -q
  git -C "$repo" config user.name "S7 ACPX Review Test"
  git -C "$repo" config user.email "s7-acpx-review-test@example.invalid"
  python3 - "$repo/README.md" <<'PY'
from pathlib import Path
import sys

Path(sys.argv[1]).write_text("s7 review test repo\n", encoding="utf-8")
PY
  git -C "$repo" add README.md
  git -C "$repo" commit -qm base
}

fake_acpx="$tmp/fake-acpx"
python3 - "$fake_acpx" <<'PY'
from pathlib import Path
import json
import os
import sys

path = Path(sys.argv[1])
path.write_text(
    """#!/usr/bin/env python3
import json
import os
import sys

args = sys.argv[1:]
reviewer = "claude" if "claude" in args else "gemini"
personas = {
    "gemini": ["P3", "P4", "P5", "P6", "P7", "P8"],
    "claude": ["P3", "P5", "P6", "P8"],
}[reviewer]
verdict = os.environ.get("S7_FAKE_REVIEW_VERDICT", "PASS")
payload = {
    "verdict": verdict,
    "personas": personas,
    "summary": f"{reviewer} fake review {verdict}.",
    "findings": [] if verdict == "PASS" else [
        {
            "severity": "p1",
            "status": "open",
            "body": "production packet is intentionally missing in fake review",
        }
    ],
}
print("prefix before json")
print(json.dumps(payload, sort_keys=True))
print("suffix after json")
""",
    encoding="utf-8",
)
path.chmod(0o755)
PY

pass_repo="$tmp/pass-repo"
make_repo "$pass_repo"
pass_head="$(git -C "$pass_repo" rev-parse HEAD)"

S7_FAKE_REVIEW_VERDICT=PASS scripts/review/f-s7/run-acpx-reviews.py \
  --root "$pass_repo" \
  --review-cwd "$pass_repo" \
  --acpx "$fake_acpx" \
  --timeout 1 >"$tmp/pass.out"

rg -n "S7 ACPX review runner: ok" "$tmp/pass.out" >/dev/null
scripts/review/f-s7/validate-reviews.py --root "$pass_repo" --expected-head "$pass_head" >/tmp/s7-run-acpx-reviews-validate.out
rg -n "fake-acpx" "$pass_repo/docs/review/f-s7/raw/bd-2v9r-gemini.command.txt" >/dev/null
rg -n "prefix before json" "$pass_repo/docs/review/f-s7/raw/bd-2v9r-claude.stdout.txt" >/dev/null
rg -n '"command": "acpx ' "$pass_repo/docs/review/f-s7/reviews/bd-2v9r-gemini.json" >/dev/null

nonpass_repo="$tmp/nonpass-repo"
make_repo "$nonpass_repo"

if S7_FAKE_REVIEW_VERDICT=NEEDS_CHANGES scripts/review/f-s7/run-acpx-reviews.py \
  --root "$nonpass_repo" \
  --review-cwd "$nonpass_repo" \
  --acpx "$fake_acpx" \
  --timeout 1 >"$tmp/nonpass.out" 2>&1; then
  echo "expected non-PASS reviews to fail" >&2
  exit 1
fi
rg -n "review verdict was NEEDS_CHANGES; not writing PASS evidence" "$tmp/nonpass.out" >/dev/null
test ! -f "$nonpass_repo/docs/review/f-s7/reviews/bd-2v9r-gemini.json"
test -f "$nonpass_repo/docs/review/f-s7/raw/bd-2v9r-gemini.nonpass.json"

dry_repo="$tmp/dry-repo"
make_repo "$dry_repo"
scripts/review/f-s7/run-acpx-reviews.py \
  --root "$dry_repo" \
  --review-cwd /Users/bkase/Documents/gbllm \
  --acpx acpx \
  --timeout 1800 \
  --dry-run >"$tmp/dry.out"

rg -n -- "--agent 'gemini --skip-trust -m gemini-3.1-pro-preview --acp'" "$tmp/dry.out" >/dev/null
rg -n "claude exec" "$tmp/dry.out" >/dev/null
rg -n "reviewer=gemini personas=P3,P4,P5,P6,P7,P8" "$tmp/dry.out" >/dev/null
rg -n "reviewer=claude personas=P3,P5,P6,P8" "$tmp/dry.out" >/dev/null
rg -n "S7 ACPX review runner: dry-run ok" "$tmp/dry.out" >/dev/null

echo "s7_run_acpx_reviews_test: ok"
