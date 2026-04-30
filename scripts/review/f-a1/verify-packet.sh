#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

target_dir="target/review/f-a1"
artifact_dir="docs/review/f-a1/artifacts"
mkdir -p "$target_dir"

cargo test -p gbf-asm --all-features
cargo run -p gbf-asm --example tiny_rom --features stub-runtime -- "$target_dir"

cmp "$target_dir/tiny_rom.gb" "$artifact_dir/tiny_rom.gb"
cmp "$target_dir/tiny_rom.lst" "$artifact_dir/tiny_rom.lst"
cmp "$target_dir/tiny_rom.sym" "$artifact_dir/tiny_rom.sym"

tmp_hash="$(mktemp)"
trap 'rm -f "$tmp_hash"' EXIT
shasum -a 256 \
  "$artifact_dir/tiny_rom.gb" \
  "$artifact_dir/tiny_rom.lst" \
  "$artifact_dir/tiny_rom.sym" > "$tmp_hash"
cmp "$tmp_hash" "$artifact_dir/tiny_rom.sha256"

printf 'F-A1 review packet verified.\n'
