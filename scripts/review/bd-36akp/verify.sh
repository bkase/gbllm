#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
rom="${1:-$root/artifacts/builds/gbllm-dense-d192-interactive.gb}"
sym="${2:-${rom%.gb}.sym}"
out="${3:-$root/.tmp/bd-36akp-acceptance}"
script="$root/scripts/review/bd-36akp/interactive-acceptance.js"
materialize="$root/scripts/review/bd-36akp/materialize.py"
debugger="$root/target/release/gbf-debug"

if [[ ! -f "$rom" ]]; then
  echo "interactive ROM not found: $rom" >&2
  exit 2
fi
if [[ ! -f "$sym" ]]; then
  echo "interactive symbols not found: $sym" >&2
  exit 2
fi

cd "$root"
cargo build --quiet --release -p gbf-debug
mkdir -p "$out"

"$debugger" init \
  --rom "$rom" \
  --sym "$sym" \
  --out "$out/boot.gbsess" \
  --replace-existing-out \
  >"$out/init-envelope.json"

"$debugger" exec \
  --in "$out/boot.gbsess" \
  --script "$script" \
  --out "$out/accepted.gbsess" \
  --timeout 3600 \
  --emit-metrics \
  --replace-existing-out \
  >"$out/exec-envelope.json"

python3 "$materialize" "$out/exec-envelope.json" "$out"
