#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"

case "$out_dir" in
  /tmp/f-b6-f-b7-closure/*|/private/tmp/f-b6-f-b7-closure/*) ;;
  *)
    echo "error: refusing to clean unsafe F_B6_F_B7_OUT_DIR '$out_dir'" >&2
    echo "hint: use a directory under /tmp/f-b6-f-b7-closure" >&2
    exit 2
    ;;
esac

if [[ "$out_dir" == "/" || "$(basename "$out_dir")" == "." || "$(basename "$out_dir")" == ".." ]]; then
  echo "error: refusing to clean unsafe F_B6_F_B7_OUT_DIR '$out_dir'" >&2
  exit 2
fi

rm -rf -- "$out_dir"
mkdir -p "$out_dir"

export F_B6_F_B7_BUILD_ID="$build_id"
export F_B6_F_B7_OUT_DIR="$out_dir"

"$SCRIPT_DIR/stage4-run.sh" dense_default
"$SCRIPT_DIR/stage4-run.sh" moe_trace
"$SCRIPT_DIR/stage5-run.sh" chunked_i16
"$SCRIPT_DIR/run-cert-verify.sh" "$out_dir/reports/stage5/chunked_i16/certs/range.cert.json"
"$SCRIPT_DIR/coverage-pack.sh"

python3 - "$out_dir/meta.json" "$build_id" <<'PY'
import json
import pathlib
import subprocess
import sys

path = pathlib.Path(sys.argv[1])
build_id = sys.argv[2]
git_rev = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
payload = {
    "build_id": build_id,
    "git_rev": git_rev,
    "fixture_set": "gbf-codegen/tests/fixtures/f_b6_f_b7",
    "driver_status": {
        "stage4": "substrate smoke plus Rust run_stage4 diagnostic-origin gates",
        "stage5": "real run_stage5 via stage5-run.sh",
        "independent_verify": "real gbf-verify range-cert verifier via bd-2phk",
    },
    "scope_status": {
        "stage6_consumption": "deferred to bd-2k0; no executable F-B8 PlanningReady consumer is claimed",
        "ci_artifact_wiring": "out of scope; no .github workflow attachment is claimed",
        "workspace_all_features": "blocked by known gbf-experiments S2 feature mutex; focused gates are used",
        "stale_script_names": "cache-replay.sh and check-\u00a720-conformance.sh are superseded by inline check-checklist.sh/verify-packet.sh checks",
    },
}
path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if [[ "${F_B6_F_B7_SKIP_TAR:-0}" != "1" ]]; then
  packet="$out_dir/closure-packet.tar.gz"
  if [[ -e "$packet" ]]; then
    rm -f "$packet"
  fi
  tmp_tar="$(dirname "$out_dir")/$(basename "$out_dir").closure-packet.tar.gz"
  tar -C "$(dirname "$out_dir")" -czf "$tmp_tar" "$(basename "$out_dir")"
  mv "$tmp_tar" "$packet"
  echo "$packet"
else
  echo "$out_dir"
fi
