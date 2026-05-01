#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

target_dir="target/review/f-a1"
artifact_dir="docs/review/f-a1/artifacts"
mkdir -p "$target_dir" "$artifact_dir"

cargo test -p gbf-asm --all-features | tee "$artifact_dir/test-output.txt"
cargo run -p gbf-asm --example tiny_rom --features stub-runtime -- "$target_dir"

cp "$target_dir/tiny_rom.gb" "$artifact_dir/tiny_rom.gb"
cp "$target_dir/tiny_rom.lst" "$artifact_dir/tiny_rom.lst"
cp "$target_dir/tiny_rom.sym" "$artifact_dir/tiny_rom.sym"

shasum -a 256 \
  "$artifact_dir/tiny_rom.gb" \
  "$artifact_dir/tiny_rom.lst" \
  "$artifact_dir/tiny_rom.sym" > "$artifact_dir/tiny_rom.sha256"

cargo tree -p gbf-asm > "$artifact_dir/cargo-tree.txt"
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check > "$artifact_dir/cargo-deny.txt"
else
  printf 'cargo-deny not installed in packet build environment.\n' > "$artifact_dir/cargo-deny.txt"
fi
printf 'No F-A1 runtime benchmark is claimed; ROM/listing/sym generation is covered by verify-packet.sh.\n' > "$artifact_dir/bench-output.txt"
printf 'Coverage was not generated for this packet; claim-to-gate.md lists executable tests instead.\n' > "$artifact_dir/coverage-summary.txt"
