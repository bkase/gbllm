#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
hardened="${1:-$root/training/artifacts/student_dense_d192}"
tokenizer="${2:-$root/training/artifacts/tinystories_bpe_1024.json}"
cared_rom="${3:-$root/artifacts/builds/gbllm-dense-d192-interactive.gb}"
expected_sha="230a3842cc44fd8a96393ffaaa75d4954001fc03e9333273393dca9606a75a7b"

for input in "$hardened/manifest.json" "$hardened/hardened.safetensors" "$tokenizer"; do
  if [[ ! -f "$input" ]]; then
    echo "required local artifact not found: $input" >&2
    exit 2
  fi
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/gbllm-bd-3mi.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

(
  cd "$root/training"
  uv run python -c \
    'from gbtrain.bridge import bridge_hardened_export; import sys; bridge_hardened_export(sys.argv[1], sys.argv[2])' \
    "$hardened" "$scratch/ckpt"
)

cd "$root"
cargo run --release -p gbf-cli -- compile \
  --profile interactive-subword-dmg \
  --checkpoint-export "$scratch/ckpt" \
  --tokenizer "$tokenizer" \
  --tokens 24 \
  --top-k 4 \
  --temperature 0.6 \
  --rng-seed 0x5eed \
  --out "$scratch/build"

actual_sha="$(shasum -a 256 "$scratch/build/rom.gb" | awk '{print $1}')"
if [[ "$actual_sha" != "$expected_sha" ]]; then
  echo "ROM SHA-256 mismatch: expected $expected_sha, got $actual_sha" >&2
  exit 1
fi

if [[ -f "$cared_rom" ]]; then
  cmp "$scratch/build/rom.gb" "$cared_rom"
  echo "byte-identical to $cared_rom"
else
  echo "cared-for ROM not present; SHA-256 gate still passed: $cared_rom" >&2
fi

python3 - "$scratch/build/build_report.json" "$scratch/build/compile_request.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1]))
request = json.load(open(sys.argv[2]))
assert report["schema"] == "gbf_interactive_subword_build_report.v1"
assert report["rom"]["sha256"] == "230a3842cc44fd8a96393ffaaa75d4954001fc03e9333273393dca9606a75a7b"
assert report["rom"]["paged_head_storage"] == "SramFull"
assert report["rom"]["weight_lowering"] == "V3"
assert request["profile"] == "interactive-subword-dmg"
PY

echo "bd-3mi compiler-path verification passed ($actual_sha)"
