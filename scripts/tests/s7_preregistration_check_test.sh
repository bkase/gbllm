#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/s7_preregistration_check.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

init_repo() {
  git init -q "$TMPDIR/repo"
  cd "$TMPDIR/repo"
  git config user.email "s7-prereg@example.test"
  git config user.name "S7 Prereg Test"
}

write_rfc() {
  local mutation="${1:-}"
  mkdir -p history/rfcs
  python3 - "$mutation" <<'PY'
import sys
from pathlib import Path

mutation = sys.argv[1]
lines = [
    "# F-S7 fixture",
    "",
    "## Pre-registered predictions",
    "",
    "```text",
    "H1: MoeTiny completes all seeds.",
    "H2: Dense matched completes all seeds.",
    "H3: matched bytes remain within tolerance.",
    "H4: MoE dominates dense on median bpc.",
    "H5: switch stats remain healthy.",
    "H6: lambda_switch sweep does not collapse.",
    "H7: gradients reach only declared tensors.",
    "H8: ExpertBlockQat burn smoke passes.",
    "H9: routed oracle agrees.",
    "H10: emulator one-token passes.",
    "Decision: ProceedToS8 if all gates confirm.",
    "```",
]
if mutation:
    lines[8] = f"H4: MoE dominates dense on median bpc {mutation}."
Path("history/rfcs/F-S7-moe-beats-dense.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
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
path = "history/rfcs/F-S7-moe-beats-dense.md"
start = 3
end = 17
lines = Path(path).read_text(encoding="utf-8").replace("\r\n", "\n").replace("\r", "\n").split("\n")
section = "\n".join(lines[start - 1:end]).strip()
payload = {"path": path, "start_line": start, "end_line": end, "section": section}
digest = "sha256:" + hashlib.sha256(
    json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
).hexdigest()
Path("fixtures/preregistration/s7.toml").write_text(
    "\n".join(
        [
            'schema = "s7_preregistration.v1"',
            f'rfc_path = "{path}"',
            f"predictions_line_start = {start}",
            f"predictions_line_end = {end}",
            f'predictions_commit = "{predictions_commit}"',
            f'predictions_section_hash = "{digest}"',
            'pass_version_S7 = "s7-prereg-test-2026-06-25"',
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
git add history/rfcs/F-S7-moe-beats-dense.md
git commit -q -m "pre-register S7 predictions"
predictions_commit="$(git rev-parse HEAD)"

expect_fail missing_pin "pin not found:"

write_pin "$predictions_commit"
git add fixtures/preregistration/s7.toml
git commit -q -m "record S7 preregistration pin"

"$SCRIPT" >/dev/null
first_hash="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path("/tmp/s7-preregistration.json").read_text())["events"][2]["detail"]["predictions_section_hash"])
PY
)"
"$SCRIPT" >/dev/null
second_hash="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path("/tmp/s7-preregistration.json").read_text())["events"][2]["detail"]["predictions_section_hash"])
PY
)"
test "$first_hash" = "$second_hash"

mkdir -p experiments/S7/profile
cat >experiments/S7/profile/matched_bytes.json <<'JSON'
{"matched_bytes_self_hash":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
JSON
"$SCRIPT" >/dev/null

mkdir -p experiments/S7/smoke
cat >experiments/S7/smoke/s7_dense_vs_moe.v1.json <<'JSON'
{"comparison_self_hash":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
JSON
"$SCRIPT" >/dev/null

mkdir -p experiments/S7/runs/MoeTiny/seed-0
cat >experiments/S7/runs/MoeTiny/seed-0/run-log.json <<'JSON'
{"run_log_self_hash":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
JSON
expect_fail uncommitted_result "first_result_commit is unset but S7 result evidence exists in the worktree"
rm -rf experiments/S7

write_rfc "MUTATED"
expect_fail mutated_predictions "offending_diff_hunk:"
grep -F "line_range=3..17" "$TMPDIR/mutated_predictions.err" >/dev/null

write_rfc
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(text.replace("predictions_line_start = 3", "predictions_line_start = 18"), encoding="utf-8")
PY
expect_fail malformed_range "predictions line range is invalid"

write_pin "$predictions_commit"
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s7.toml")
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

path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('predictions_commit = "', 'predictions_commit = "BAD', 1),
    encoding="utf-8",
)
PY
expect_fail malformed_commit "predictions_commit must be a lowercase 40-character git commit id"

write_pin "$predictions_commit"
side_commit="$(git commit-tree HEAD^{tree} -m "side prereg commit")"
write_pin "$side_commit"
expect_fail side_predictions_not_ancestor "predictions_commit must be an ancestor of HEAD"

write_pin "$predictions_commit"
python3 - "$predictions_commit" "$side_commit" <<'PY'
from pathlib import Path
import sys

predictions_commit, side_commit = sys.argv[1:3]
path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace(
        f'rfc_revision = "{predictions_commit}"',
        f'rfc_revision = "{side_commit}"',
    ),
    encoding="utf-8",
)
PY
expect_fail side_rfc_revision_not_ancestor "rfc_revision must be an ancestor of HEAD"

write_pin "$predictions_commit"
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('pass_version_S7 = "s7-prereg-test-2026-06-25"', 'pass_version_S7 = "fixture"'),
    encoding="utf-8",
)
PY
expect_fail placeholder_pass_version "pass_version_S7 must be finalized"

write_pin "$predictions_commit"
python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('pass_version_S7 = "s7-prereg-test-2026-06-25"', 'pass_version_S7 = "S7 Final"'),
    encoding="utf-8",
)
PY
expect_fail malformed_pass_version "pass_version_S7 must be semver or an s7-* final pin id"

write_pin "$predictions_commit"
mkdir -p experiments/S7/dense-vs-moe
cat >experiments/S7/dense-vs-moe/comparison.json <<'JSON'
{"comparison_self_hash":"sha256:1111111111111111111111111111111111111111111111111111111111111111"}
JSON
git add experiments/S7/dense-vs-moe/comparison.json
git commit -q -m "add first S7 result"
first_result_commit="$(git rev-parse HEAD)"

write_pin "$predictions_commit" "$first_result_commit"
"$SCRIPT" >/dev/null

mkdir -p experiments/S7/frontier
cat >experiments/S7/frontier/frontier.json <<'JSON'
{"frontier_self_hash":"sha256:2222222222222222222222222222222222222222222222222222222222222222"}
JSON
git add experiments/S7/frontier/frontier.json
git commit -q -m "add later S7 result"
later_result_commit="$(git rev-parse HEAD)"
write_pin "$predictions_commit" "$later_result_commit"
expect_fail later_result_not_earliest "first_result_commit is not the earliest S7 result artifact commit"

write_pin "$predictions_commit" "$first_result_commit"
git add fixtures/preregistration/s7.toml
git commit -q -m "record S7 result commit after first result"
"$SCRIPT" >/dev/null

python3 - <<'PY'
from pathlib import Path

path = Path("fixtures/preregistration/s7.toml")
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('pass_version_S7 = "s7-prereg-test-2026-06-25"', 'pass_version_S7 = "s7-prereg-test-mutated"'),
    encoding="utf-8",
)
PY
expect_fail late_pin_scope "pin commits after first_result_commit may only update first_result_commit"

echo "[S7 PREREG TEST] all preregistration check scenarios passed"
