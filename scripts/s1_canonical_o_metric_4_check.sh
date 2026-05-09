#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/fixtures/corpora/tinystories.toml"
RAW_ROOT=""
VERIFY_ONLY=0
SELF_TEST=0

usage() {
  cat <<'USAGE'
usage: scripts/s1_canonical_o_metric_4_check.sh [options]

Verify the canonical TinyStories O-metric-4 shuffle permutation/pin oracle.
The script downloads or verifies only the validation split, then runs the exact
ignored cargo tests that require canonical validation bytes.

options:
  --manifest PATH    TinyStories manifest path
  --raw-root PATH    Override raw corpus directory
  --verify-only      Do not download; fail if validation bytes are absent/stale
  --self-test        Exercise split-scoped downloader behavior without network
  -h, --help         Show this help
USAGE
}

while (($#)); do
  case "$1" in
    --manifest)
      MANIFEST="${2:?--manifest requires a path}"
      shift 2
      ;;
    --raw-root)
      RAW_ROOT="${2:?--raw-root requires a path}"
      shift 2
      ;;
    --verify-only)
      VERIFY_ONLY=1
      shift
      ;;
    --self-test)
      SELF_TEST=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ((SELF_TEST)); then
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT
  python3 - "$tmpdir" <<'PY'
from pathlib import Path
import hashlib
import sys

root = Path(sys.argv[1])
raw = root / "raw"
raw.mkdir()
val = b"canonical validation fixture bytes"
(raw / "val.bytes").write_bytes(val)
val_sha = "sha256:" + hashlib.sha256(val).hexdigest()

(root / "manifest.toml").write_text(
    f'''
schema = "tinystories_manifest.v1"
schema_version = "1.0.0"
corpus_id = "tinystories-test"
dataset_version = "fixture"
source_name = "fixture"
source_url = "https://example.invalid/tinystories"
train_path = "train.bytes"
val_path = "val.bytes"
train_sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
val_sha256 = "{val_sha}"
raw_root = "raw"
raw_byte_policy = "fixture"
story_separator = "<|endoftext|>"
s1_policy = "fixture"
deferred_scope = []

[source]
name = "fixture"
url = "https://example.invalid/tinystories"
dataset_card_url = "https://example.invalid/card"
license = "fixture"
license_url = "https://example.invalid/license"
downloaded_at = "2026-05-09"
decompression = "none"

[splits.train]
role = "train"
url = "https://example.invalid/train.bytes"
local_filename = "train.bytes"
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
byte_length = 0
story_count = 0

[splits.validation]
role = "validation"
url = "https://example.invalid/val.bytes"
local_filename = "val.bytes"
sha256 = "{val_sha}"
byte_length = {len(val)}
story_count = 1
'''.lstrip(),
    encoding="utf-8",
)
PY
  python3 "$ROOT/scripts/download_tinystories.py" \
    --manifest "$tmpdir/manifest.toml" \
    --split validation \
    --verify-only
  if python3 "$ROOT/scripts/download_tinystories.py" \
    --manifest "$tmpdir/manifest.toml" \
    --split train \
    --verify-only 2>"$tmpdir/train.err"; then
    echo "expected train-only verification to fail with missing train split" >&2
    exit 1
  fi
  grep -q "train.bytes: missing" "$tmpdir/train.err"
  echo "[O-METRIC-4] self-test ok"
  exit 0
fi

download_args=(--manifest "$MANIFEST" --split validation)
if [[ -n "$RAW_ROOT" ]]; then
  download_args+=(--raw-root "$RAW_ROOT")
fi
if ((VERIFY_ONLY)); then
  download_args+=(--verify-only)
fi

python3 "$ROOT/scripts/download_tinystories.py" "${download_args[@]}"

cargo test -p gbf-experiments --test oracle -- \
  --ignored --exact o_metric_4_canonical_tinystories_shuffle_matches_manifest_pin
cargo test -p gbf-experiments --test tinystories_shuffle_pin -- \
  --ignored --exact tinystories_shuffle_pin_recomputes_from_canonical_validation_bytes
