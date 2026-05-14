#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/s1_preregistration_check.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

git_init() {
  local name="${1:-repo}"
  git init -q "$TMPDIR/$name"
  cd "$TMPDIR/$name"
  git config user.email "s1-prereg@example.test"
  git config user.name "S1 Prereg Test"
}

write_report() {
  local predictions_commit="$1"
  local first_result_commit="$2"
  local prediction_suffix="${3:-}"
  python3 - "$predictions_commit" "$first_result_commit" "$prediction_suffix" <<'PY'
import hashlib
import json
import os
import sys
from pathlib import Path

predictions_commit, first_result_commit, suffix = sys.argv[1:4]
report_path = Path(os.environ.get("REPORT_PATH", "docs/experiments/S1-report.md"))
predictions = "\n".join([
    "### H1 Plumbing",
    "",
    "- Predicted: finite loss and gradient norms.",
    "- Falsification: any non-finite loss or gradient refutes H1.",
    "",
    "### H2 Capacity",
    "",
    "- Predicted: every seed beats the trigram baseline by 0.05 bpc.",
    "- Falsification: any seed failing that margin refutes H2.",
    "",
    "### H3 Sequence-state utility",
    "",
    "- Predicted: seed 0 negative-test delta exceeds 2.0 bpc.",
    "- Falsification: delta <= 2.0 refutes H3.",
    "",
    "### H4 Phase A cleanliness",
    "",
    "- Predicted: seed 0 tensor payload hashes match.",
    "- Falsification: payload hash mismatch refutes H4.",
    "",
    "### H5 Measurement",
    "",
    "- Predicted: metric_oracle_passed = true.",
    "- Falsification: metric_oracle_passed = false refutes H5.",
    "",
    "D6 per-seed strict pass criterion:",
    "",
    "- Seeds {0,1,2,3,4} must be covered.",
    "",
    "Prediction-status rule:",
    "",
    "- Predicted ranges become refutations only when repeated in Falsification.",
])
if suffix:
    predictions += "\n" + suffix

def canon(value):
    return json.dumps(value.strip(), sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()

front = {
    "schema": "s1_report.v1",
    "s1_outcome": None,
    "decision": "NotYetRun",
    "baseline_self_hash": None,
    "per_seed_artifacts": [
        {
            "seed": seed,
            "completion": {"kind": "not_reached"},
            "checkpoint_self_hash": None,
            "run_log_self_hash": None,
            "score_self_hash": None,
            "negative_self_hash": None,
            "ablation_self_hash": None,
        }
        for seed in range(5)
    ],
    "generated_at": "2026-05-09T18:30:00Z",
    "rfc_revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "predictions_section_hash": "sha256:" + hashlib.sha256(canon(predictions)).hexdigest(),
    "predictions_commit": None if predictions_commit == "null" else predictions_commit,
    "first_result_commit": None if first_result_commit == "null" else first_result_commit,
    "report_self_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
}
body = (
    "# S1 Report\n\n"
    "## Pre-registered predictions\n\n"
    f"{predictions}\n\n"
    "## Observed\n\n"
    "Populated by F-S1.29 after the run completes.\n"
)
stripped = dict(front)
stripped.pop("generated_at")
stripped.pop("report_self_hash")
preimage = (
    b"gbf:gbf-experiments:ReportFile:s1_report.v1:1\0"
    + json.dumps(stripped, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    + b"\0"
    + body.encode()
)
front["report_self_hash"] = "sha256:" + hashlib.sha256(preimage).hexdigest()
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(
    "---\n" + json.dumps(front, indent=2) + "\n---\n" + body,
    encoding="utf-8",
)
PY
}

expect_fail() {
  local label="$1"
  local expected="$2"
  local report="${3:-docs/experiments/S1-report.md}"
  set +e
  "$SCRIPT" --report "$report" >"$TMPDIR/$label.out" 2>"$TMPDIR/$label.err"
  local status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "expected $label to fail" >&2
    cat "$TMPDIR/$label.err" >&2
    exit 1
  fi
  grep -F "$expected" "$TMPDIR/$label.err" >/dev/null
}

git_init

write_report null null
git add docs/experiments/S1-report.md
git commit -q -m "pre-register predictions"
predictions_commit="$(git rev-parse HEAD)"
"$SCRIPT" --report docs/experiments/S1-report.md >/dev/null

mkdir -p experiments/S1/checkpoints/seed-0
cat >experiments/S1/checkpoints/seed-0/metadata.json <<'JSON'
{
  "schema": "s1_checkpoint.v1",
  "checkpoint_self_hash": "sha256:1111111111111111111111111111111111111111111111111111111111111111"
}
JSON
git add experiments/S1/checkpoints/seed-0/metadata.json
git commit -q -m "add first result"
first_result_commit="$(git rev-parse HEAD)"

write_report "$predictions_commit" "$first_result_commit"
git add docs/experiments/S1-report.md
git commit -q -m "record prereg commits"
"$SCRIPT" --report docs/experiments/S1-report.md >/dev/null

write_report "$first_result_commit" "$first_result_commit"
expect_fail equal_commits "predictions_commit must be strictly before first_result_commit"

write_report null "$first_result_commit"
expect_fail first_result_without_predictions "predictions_commit is required once first_result_commit is set"

mkdir -p experiments/S1/checkpoints/seed-1
cat >experiments/S1/checkpoints/seed-1/metadata.json <<'JSON'
{
  "schema": "s1_checkpoint.v1",
  "checkpoint_self_hash": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
}
JSON
git add experiments/S1/checkpoints/seed-1/metadata.json
git commit -q -m "add later result"
later_result_commit="$(git rev-parse HEAD)"

write_report "$predictions_commit" "$later_result_commit"
expect_fail later_result_not_earliest "first_result_commit is not the earliest S1 result artifact commit"
grep -F "expected_earliest_result_commit=" "$TMPDIR/later_result_not_earliest.err" >/dev/null
grep -F "observed_front_matter_first_result_commit=" "$TMPDIR/later_result_not_earliest.err" >/dev/null

write_report "$predictions_commit" "$first_result_commit" "- Edited after results."
expect_fail edited_after_results "does not match predictions_commit section"
grep -F "expected_from_predictions_commit=" "$TMPDIR/edited_after_results.err" >/dev/null
grep -F "observed_front_matter=" "$TMPDIR/edited_after_results.err" >/dev/null

write_report "$predictions_commit" "$first_result_commit"
python3 - <<'PY'
import json
from pathlib import Path
path = Path("docs/experiments/S1-report.md")
text = path.read_text()
front_raw, body = text[4:].split("\n---\n", 1)
front = json.loads(front_raw)
front["predictions_section_hash"] = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
path.write_text("---\n" + json.dumps(front, indent=2) + "\n---\n" + body)
PY
expect_fail hash_drift "predictions_section_hash mismatch in current report"
grep -F "expected_from_body=" "$TMPDIR/hash_drift.err" >/dev/null
grep -F "observed_front_matter=" "$TMPDIR/hash_drift.err" >/dev/null

write_report "$predictions_commit" "$first_result_commit"
python3 - <<'PY'
import hashlib
import json
from pathlib import Path
path = Path("docs/experiments/S1-report.md")
text = path.read_text()
front_raw, body = text[4:].split("\n---\n", 1)
front = json.loads(front_raw)
marker = "## Pre-registered predictions\n\n"
start = body.index(marker) + len(marker)
end = body.index("\n## Observed\n", start)
section = body[start:end].strip()
front["predictions_section_hash"] = "sha256:" + hashlib.sha256(section.encode()).hexdigest()
path.write_text("---\n" + json.dumps(front, indent=2) + "\n---\n" + body)
PY
expect_fail raw_hash_drift "predictions_section_hash mismatch in current report"

write_report "$(git rev-parse HEAD)" "$first_result_commit"
expect_fail not_ancestor "predictions commit not an ancestor of first result"

git_init custom-report-repo

mkdir -p experiments/S1/checkpoints/seed-0
cat >experiments/S1/checkpoints/seed-0/metadata.json <<'JSON'
{
  "schema": "s1_checkpoint.v1",
  "checkpoint_self_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
JSON
git add experiments/S1/checkpoints/seed-0/metadata.json
git commit -q -m "existing toy0 result history"

custom_report="reports/S1/custom-report.md"
REPORT_PATH="$custom_report" write_report null null
git add "$custom_report"
git commit -q -m "custom pre-register predictions"
custom_predictions_commit="$(git rev-parse HEAD)"

mkdir -p experiments/S1-toy1/checkpoints/seed-0
cat >experiments/S1-toy1/checkpoints/seed-0/metadata.json <<'JSON'
{
  "schema": "s1_checkpoint.v1",
  "checkpoint_self_hash": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
}
JSON
git add experiments/S1-toy1/checkpoints/seed-0/metadata.json
git commit -q -m "custom report first result"
custom_first_result_commit="$(git rev-parse HEAD)"

REPORT_PATH="$custom_report" write_report "$custom_predictions_commit" "$custom_first_result_commit"
git add "$custom_report"
git commit -q -m "custom report records result commit"
"$SCRIPT" --report "$custom_report" --artifact-dir experiments/S1-toy1 >/dev/null

git_init squash-carrier-repo

write_report null null
git add docs/experiments/S1-report.md
git commit -q -m "pre-register predictions before squash"
squash_predictions_commit="$(git rev-parse HEAD)"

mkdir -p experiments/S1/checkpoints/seed-0
cat >experiments/S1/checkpoints/seed-0/metadata.json <<'JSON'
{
  "schema": "s1_checkpoint.v1",
  "checkpoint_self_hash": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
}
JSON
git add experiments/S1/checkpoints/seed-0/metadata.json
git commit -q -m "add audited first result before squash"
squash_first_result_commit="$(git rev-parse HEAD)"

write_report "$squash_predictions_commit" "$squash_first_result_commit"
git add docs/experiments/S1-report.md
git commit -q -m "record audited report before squash"
audited_report_commit="$(git rev-parse HEAD)"

git checkout --orphan squash-main >/dev/null 2>&1
git rm -rf . >/dev/null 2>&1 || true
rm -rf docs experiments
git checkout "$audited_report_commit" -- docs/experiments/S1-report.md experiments/S1
git add docs/experiments/S1-report.md experiments/S1
git commit -q -m "squash audited report and artifacts"
"$SCRIPT" --report docs/experiments/S1-report.md >/dev/null

echo "[PREREG TEST] all preregistration check scenarios passed"
