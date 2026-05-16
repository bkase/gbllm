#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${S3_MANIFEST_VERIFY_NDJSON:-/tmp/s3-manifest-verify.json}"

cd "$repo_root"

cargo run -q -p gbf-data --example s3_manifest_verify -- \
    fixtures/corpora/tinystories.v2.toml \
    "$out"

if [[ ! -s "$out" ]]; then
    echo "s3_manifest_verify: expected non-empty NDJSON at $out" >&2
    exit 1
fi

line_count="$(wc -l < "$out" | tr -d '[:space:]')"
if [[ "$line_count" -ne 8 ]]; then
    echo "s3_manifest_verify: expected 8 sha verification records, saw $line_count" >&2
    exit 1
fi

echo "s3_manifest_verify: wrote $out ($line_count records)"
