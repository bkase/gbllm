#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/s4_preregistration_check.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

init_repo() {
  git init -q "$TMPDIR/repo"
  cd "$TMPDIR/repo"
  git config user.email "s4-prereg@example.test"
  git config user.name "S4 Prereg Test"
}

write_rfc() {
  local mutation="${1:-}"
  mkdir -p history/rfcs
  python3 - "$mutation" <<'PY'
import sys
from pathlib import Path

mutation = sys.argv[1]
lines = [
    "# F-S4 fixture",
    "",
    "## H1 Corpus integrity",
    "",
    "```text",
    "Statement:",
    "  fixture statement",
    "",
    "Predicted:",
    "  manifest_self_hash round-trips",
    "",
    "Falsification:",
    "  manifest_self_hash mismatch => Refuted",
    "",
    "Verdict:",
    "  Confirmed otherwise.",
    "```",
]
if mutation:
    lines[9] = f"  manifest_self_hash round-trips {mutation}"
Path("history/rfcs/F-S4-gutenberg-promotion.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
}

write_pin() {
  local predictions_commit="$1"
  local first_result_commit="${2:-}"
  mkdir -p fixtures/preregistration
  python3 - "$predictions_commit" "$first_result_commit" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

predictions_commit, first_result_commit = sys.argv[1:3]
path = "history/rfcs/F-S4-gutenberg-promotion.md"
start = 3
end = 17
lines = Path(path).read_text(encoding="utf-8").replace("\r\n", "\n").replace("\r", "\n").split("\n")
section = "\n".join(lines[start - 1:end]).strip()
payload = {"path": path, "start_line": start, "end_line": end, "section": section}
digest = "sha256:" + hashlib.sha256(
    json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
).hexdigest()
Path("fixtures/preregistration/s4.toml").write_text(
    "\n".join(
        [
            'schema = "s4_preregistration.v1"',
            f'rfc_path = "{path}"',
            f"predictions_line_start = {start}",
            f"predictions_line_end = {end}",
            f'predictions_commit = "{predictions_commit}"',
            f'predictions_section_hash = "{digest}"',
            'pass_version_S4 = "fixture"',
            f'rfc_revision = "{predictions_commit}"',
            f'first_result_commit = "{first_result_commit}"',
            "",
        ]
    ),
    encoding="utf-8",
)
PY
}

expect_fail() {
  local label="$1"
  local expected="$2"
  set +e
  "$SCRIPT" >"$TMPDIR/$label.out" 2>"$TMPDIR/$label.err"
  local status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "expected $label to fail" >&2
    cat "$TMPDIR/$label.err" >&2
    exit 1
  fi
  grep -F "$expected" "$TMPDIR/$label.err" >/dev/null
}

init_repo
write_rfc
git add history/rfcs/F-S4-gutenberg-promotion.md
git commit -q -m "pre-register S4 predictions"
predictions_commit="$(git rev-parse HEAD)"
write_pin "$predictions_commit"
git add fixtures/preregistration/s4.toml
git commit -q -m "record S4 preregistration pin"

"$SCRIPT" >/dev/null
first_hash="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path("/tmp/s4-preregistration.json").read_text())["events"][2]["detail"]["predictions_section_hash"])
PY
)"
"$SCRIPT" >/dev/null
second_hash="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path("/tmp/s4-preregistration.json").read_text())["events"][2]["detail"]["predictions_section_hash"])
PY
)"
test "$first_hash" = "$second_hash"

mkdir -p experiments/S4/seed-0
cat >experiments/S4/seed-0/uncommitted-score.json <<'JSON'
{"score_self_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
JSON
expect_fail uncommitted_result "first_result_commit is unset but S4 result evidence exists in the worktree"
rm -rf experiments/S4

write_rfc "MUTATED"
expect_fail mutated_predictions "offending_diff_hunk:"
grep -F "line_range=3..17" "$TMPDIR/mutated_predictions.err" >/dev/null

write_rfc
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s4.toml")
text = path.read_text(encoding="utf-8")
path.write_text(text.replace("predictions_line_start = 3", "predictions_line_start = 18"), encoding="utf-8")
PY
expect_fail malformed_range "predictions line range is invalid"

write_pin "$predictions_commit"
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s4.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace(
        'predictions_section_hash = "sha256:',
        'predictions_section_hash = "sha256:XYZ',
    ),
    encoding="utf-8",
)
PY
expect_fail malformed_hash "predictions_section_hash must be sha256:<64 lowercase hex>"

write_pin "$predictions_commit"
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s4.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('predictions_commit = "', 'predictions_commit = "BAD', 1),
    encoding="utf-8",
)
PY
expect_fail malformed_commit "predictions_commit must be a lowercase 40-character git commit id"

write_pin "$predictions_commit"
mkdir -p experiments/S4/seed-0
cat >experiments/S4/seed-0/score.json <<'JSON'
{"score_self_hash":"sha256:1111111111111111111111111111111111111111111111111111111111111111"}
JSON
git add experiments/S4/seed-0/score.json
git commit -q -m "add first S4 result"
first_result_commit="$(git rev-parse HEAD)"
write_pin "$predictions_commit" "$first_result_commit"
git add fixtures/preregistration/s4.toml
git commit -q -m "record S4 result commit too late"
expect_fail late_pin_touch "commit touching fixtures/preregistration/s4.toml is not an ancestor"

echo "[S4 PREREG TEST] all preregistration check scenarios passed"
