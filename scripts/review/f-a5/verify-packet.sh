#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

OUT_DIR="${1:-target/review/f-a5}"

cargo fmt --check --all
cargo test -p gbf-abi -p gbf-asm -p gbf-runtime
cargo run -p gbf-runtime --example demo_bank0_rom -- "$OUT_DIR"
cargo run -p gbf-runtime --example render_demo_screen -- "$OUT_DIR/demo-screen.png"

expected_hash="$(sed -n 's/.*`\([0-9a-f]\{64\}\)`.*/\1/p' artifacts/calibration/PINNED_HASH_HISTORY.md | head -n 1)"
if [[ -z "$expected_hash" ]]; then
  echo "no pinned runtime_nucleus_hash found in artifacts/calibration/PINNED_HASH_HISTORY.md" >&2
  exit 1
fi

actual_hash="$(tr -d '\n' < "$OUT_DIR/runtime_nucleus_hash.txt")"
if [[ "$expected_hash" != "$actual_hash" ]]; then
  echo "runtime_nucleus_hash mismatch: expected $expected_hash, got $actual_hash" >&2
  exit 1
fi

if ! cmp -s "$OUT_DIR/bank0_section_sizes.json" docs/review/f-a5/bank0-section-sizes.json; then
  echo "bank0 section sizes are stale; refresh docs/review/f-a5/bank0-section-sizes.json" >&2
  diff -u docs/review/f-a5/bank0-section-sizes.json "$OUT_DIR/bank0_section_sizes.json" >&2 || true
  exit 1
fi

if ! cmp -s "$OUT_DIR/demo-screen.png" docs/review/f-a5/demo-screen.png; then
  echo "demo screen PNG is stale; refresh docs/review/f-a5/demo-screen.png" >&2
  exit 1
fi
