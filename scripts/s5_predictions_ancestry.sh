#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_PATH="${S5_PREDICTIONS_ANCESTRY_REPORT:-}"

usage() {
  cat <<'USAGE'
Usage: scripts/s5_predictions_ancestry.sh [--report PATH] [--self-test]

Verifies RP-Predictions-Ancestry when an s5_report.v1 JSON/Markdown report is
available: predictions_commit must be an ancestor of first_result_commit.

If no report is available yet, this RFC-named gate verifies the committed RFC
substrate and prints a clear substrate-only note instead of claiming live
closure-report producer execution.
USAGE
}

self_test=0
while (($#)); do
  case "$1" in
    --report)
      shift
      if (($# == 0)); then
        echo "s5_predictions_ancestry.sh: --report requires a path" >&2
        exit 2
      fi
      REPORT_PATH="$1"
      ;;
    --self-test)
      self_test=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "s5_predictions_ancestry.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

extract_commits() {
  python3 - "$1" <<'PY'
import json
import re
import sys
from pathlib import Path

text = Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(r"\{.*\}", text, re.S)
if not match:
    raise SystemExit("report does not contain a JSON object")
payload = json.loads(match.group(0))
predictions = payload.get("predictions_commit")
first_result = payload.get("first_result_commit")
if not predictions or not first_result:
    raise SystemExit("report must contain predictions_commit and first_result_commit")
print(predictions)
print(first_result)
PY
}

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  grep -F "scripts/s5_predictions_ancestry.sh" "$ROOT/history/rfcs/F-S5-pick-and-fit.md" >/dev/null
  grep -F "predictions_commit" "$ROOT/history/rfcs/F-S5-pick-and-fit.md" >/dev/null
  echo "[S5 PREDICTIONS ANCESTRY] self-test PASS"
  exit 0
fi

if [[ -n "$REPORT_PATH" ]]; then
  mapfile -t commits < <(extract_commits "$REPORT_PATH")
  predictions_commit="${commits[0]}"
  first_result_commit="${commits[1]}"
  git -C "$ROOT" merge-base --is-ancestor "$predictions_commit" "$first_result_commit"
  echo "S5 predictions ancestry PASS report=$REPORT_PATH predictions_commit=$predictions_commit first_result_commit=$first_result_commit"
  exit 0
fi

grep -F "RP-Predictions-Ancestry" "$ROOT/history/rfcs/F-S5-pick-and-fit.md" >/dev/null
grep -F "scripts/s5_predictions_ancestry.sh" "$ROOT/history/rfcs/F-S5-pick-and-fit.md" >/dev/null

cat <<'NOTE'
S5 predictions ancestry PASS substrate=RFC RP-Predictions-Ancestry wording
SUBSTRATE_ONLY: no s5_report.v1 path was provided, so no live git ancestry was checked.
owner: closure report producer for bd-36y1/bd-1cdu final same-PR close.
NOTE
