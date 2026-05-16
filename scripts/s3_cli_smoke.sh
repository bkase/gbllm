#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="${TMPDIR:-/tmp}"
RUN_DIR="$(mktemp -d "$TMPDIR/s3-cli.XXXXXX")"
FEATURES="s3-full"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

cd "$ROOT"

verbs=(
  replay-full
  replay-fallback
  verify-determinism
  normalize-corpus
  fit-baseline
  export-bundle
  export-artifact
  oracle-agreement
  oracle-re-run
  report
)

for verb in "${verbs[@]}"; do
  cargo run -q -p gbf-cli --features "$FEATURES" -- s3 "$verb" --help >/dev/null
done

run_gbf() {
  local verb="$1"
  local expect="$2"
  shift 2
  local events="$RUN_DIR/${verb}.ndjson"
  rm -f "$events"
  if [[ "$expect" == "success" ]]; then
    cargo run -q -p gbf-cli --features "$FEATURES" -- \
      --capture-events "$events" \
      "$@"
  else
    set +e
    cargo run -q -p gbf-cli --features "$FEATURES" -- \
      --capture-events "$events" \
      "$@" >/dev/null 2>"$RUN_DIR/${verb}.stderr"
    local status=$?
    set -e
    if [[ "$status" -eq 0 ]]; then
      echo "expected $verb to fail under $FEATURES" >&2
      exit 1
    fi
  fi
  python3 - "$events" "$verb" "$expect" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
verb = sys.argv[2]
expect = sys.argv[3]
events = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
names = [event.get("fields", {}).get("event_name") for event in events]
assert "s3::cli::start" in names, names
assert "s3::cli::done" in names, names
done = [event for event in events if event.get("fields", {}).get("event_name") == "s3::cli::done"][-1]
assert done["fields"]["verb"] == verb, done
assert done["fields"]["exit_code"] == (0 if expect == "success" else 1), done
PY
}

REPLAY="$RUN_DIR/replay-full.json"
FALLBACK="$RUN_DIR/replay-fallback.json"
DETERMINISM="$RUN_DIR/determinism.json"
CHARSET="$RUN_DIR/charset.json"
CHARSET_CLI="$RUN_DIR/charset-cli.json"
BASELINE="$RUN_DIR/baseline.json"
BASELINE_CLI="$RUN_DIR/baseline-cli.json"
BUNDLE="$RUN_DIR/bundle.json"
BUNDLE_META="$RUN_DIR/bundle-metadata.json"
BUNDLE_CLI="$RUN_DIR/bundle-cli.json"
ARTIFACT="$RUN_DIR/artifact.bin"
ARTIFACT_META="$RUN_DIR/artifact-metadata.json"
ARTIFACT_CLI="$RUN_DIR/artifact-cli.json"
AGREEMENT="$RUN_DIR/agreement.json"
AGREEMENT_CLI="$RUN_DIR/agreement-cli.json"
ORACLE_RERUN="$RUN_DIR/oracle-re-run.json"
ORACLE_RERUN_CLI="$RUN_DIR/oracle-re-run-cli.json"
REPORT="$RUN_DIR/S3-report.md"
REPORT_CLI="$RUN_DIR/report-cli.json"

run_gbf replay-full success \
  s3 replay-full --output "$REPLAY"
run_gbf replay-fallback failure \
  s3 replay-fallback --output "$FALLBACK"
run_gbf verify-determinism success \
  s3 verify-determinism --seed-list 0 --output "$DETERMINISM"
run_gbf normalize-corpus success \
  s3 normalize-corpus --output "$CHARSET" --evidence-output "$CHARSET_CLI"
run_gbf fit-baseline success \
  s3 fit-baseline --output "$BASELINE" --evidence-output "$BASELINE_CLI"
run_gbf export-bundle success \
  s3 export-bundle --bundle-output "$BUNDLE" --metadata-output "$BUNDLE_META" --evidence-output "$BUNDLE_CLI"
run_gbf export-artifact success \
  s3 export-artifact --artifact-output "$ARTIFACT" --metadata-output "$ARTIFACT_META" --evidence-output "$ARTIFACT_CLI"
run_gbf oracle-agreement success \
  s3 oracle-agreement --output "$AGREEMENT" --evidence-output "$AGREEMENT_CLI"
run_gbf oracle-re-run success \
  s3 oracle-re-run --output "$ORACLE_RERUN" --evidence-output "$ORACLE_RERUN_CLI"
run_gbf report success \
  s3 report --replay-full "$REPLAY" --export-bundle "$BUNDLE_CLI" --export-artifact "$ARTIFACT_CLI" --oracle-agreement "$AGREEMENT_CLI" --oracle-re-run "$ORACLE_RERUN_CLI" --normalize-corpus "$CHARSET_CLI" --fit-baseline "$BASELINE_CLI" --output "$REPORT" --evidence-output "$REPORT_CLI"

python3 - "$REPLAY" "$DETERMINISM" "$CHARSET_CLI" "$BASELINE_CLI" "$BUNDLE_CLI" "$ARTIFACT_CLI" "$AGREEMENT_CLI" "$ORACLE_RERUN_CLI" "$REPORT" "$REPORT_CLI" <<'PY'
import json
import sys
from pathlib import Path

expected = {
    1: "s3_replay_full_cli.v1",
    2: "s3_verify_determinism_cli.v1",
    3: "s3_charset_normalize_cli.v1",
    4: "s3_fit_baseline_cli.v1",
    5: "s3_export_bundle_cli.v1",
    6: "s3_export_artifact_cli.v1",
    7: "s3_oracle_agreement_cli.v1",
    8: "s3_oracle_re_run_cli.v1",
    10: "s3_report_cli.v1",
}
for index, schema in expected.items():
    payload = json.loads(Path(sys.argv[index]).read_text())
    assert payload["schema"] == schema, (sys.argv[index], payload.get("schema"))

report_cli = json.loads(Path(sys.argv[10]).read_text())
consumed = {row["evidence_kind"] for row in report_cli["consumed_evidence"]}
assert {
    "replay-full",
    "export-bundle",
    "export-artifact",
    "oracle-agreement",
    "oracle-re-run",
    "normalize-corpus",
    "fit-baseline",
} <= consumed, consumed

report = Path(sys.argv[9]).read_text()
assert '"schema":"s3_report.v1"' in report
assert "## Pre-registered predictions" in report
PY

printf 'S3 CLI smoke PASS dir=%s\n' "$RUN_DIR"
