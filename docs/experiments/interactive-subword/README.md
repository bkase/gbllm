# Interactive subword cartridge

> **Status: current production artifact and acceptance evidence.** The complete
> training, model, lowering, and runtime explanation is in the
> [repository README](../../../README.md).

The dense d192/V1024 cartridge now accepts a prompt through the Game Boy
joypad, performs the deployed byte-BPE on the cartridge, and generates the
same full-u16 token sequence as the host integer model.

## Play it

Local artifact:

```text
artifacts/builds/gbllm-dense-d192-interactive.gb
```

The ROM is an 8 MiB MBC5+RAM cartridge and requires 8 KiB cartridge SRAM.
Controls are:

- D-pad: move over the 4x19 keyboard.
- A: type the selected character.
- B: delete one character.
- START: tokenize and generate.

The prompt row accepts 20 ASCII bytes. The ROM initializes its own deterministic
`0x5EED` RNG seed; it does not require debugger or emulator memory pokes.

SHA-256:

```text
230a3842cc44fd8a96393ffaaa75d4954001fc03e9333273393dca9606a75a7b
```

## Independent debugger acceptance

Run:

```bash
scripts/review/bd-36akp/verify.sh
```

The `gbf-debug` script entered `Once upon a time` using JOYP press/release
frames only. The on-cartridge tokenizer produced exactly:

```text
[435, 443, 258, 402]
```

It then generated 24 device tokens and rendered:

```text
, there was a little
 girl named Lucy. Sh
e loved to play with
 her ball, kicking i
t.
One
```

Measured DMG timing:

| Phase | Worst M-cycles | Seconds |
|---|---:|---:|
| BPE encode, whole prompt | 370,464 | 0.353 |
| Prefill, per BPE token | 23,220,738 | 22.145 |
| First sampled token from final prefill logits | 4,691 | 0.004 |
| Steady generated token | 23,050,750 | 21.983 |

The generated-token ceiling is 30 seconds. Full debugger evidence is in
`acceptance-result.json`; `transcript.txt` and `framebuffer.pgm` are materialized
from the same successful session.

## Rebuild

The production entry point is `gbf compile`; the benchmark emitter is only a
compatibility wrapper. Starting from a hardened MLX export, first
create the Rust checkpoint layout with the Python bridge:

```bash
mkdir -p /tmp/gbllm-interactive-build
PYTHONPATH=training training/.venv/bin/python -c \
  "from gbtrain.bridge import bridge_hardened_export; \
bridge_hardened_export( \
  'training/artifacts/student_dense_d192', \
  '/tmp/gbllm-interactive-build/ckpt')"

cargo run --release -p gbf-cli -- compile \
  --profile interactive-subword-dmg \
  --checkpoint-export /tmp/gbllm-interactive-build/ckpt \
  --tokenizer training/artifacts/tinystories_bpe_1024.json \
  --tokens 24 \
  --top-k 4 \
  --temperature 0.6 \
  --rng-seed 0x5eed \
  --out artifacts/builds/gbllm-dense-d192-interactive
```

That output packet contains:

- `rom.gb` and `rom.sym` — the cartridge and debugger symbols;
- `build_report.json` — verified input hashes, actual topology, lowering,
  storage, sampler, ROM hash, and honest stage coverage;
- `compile_request.json` — the requested profile, inputs, and knobs.

For the pinned dense d192 export and tokenizer, `rom.gb` must have the SHA-256
shown above and must compare byte-for-byte equal to the cared-for
`artifacts/builds/gbllm-dense-d192-interactive.gb`.

## Source-of-truth boundary

The code path that actually produces the ROM is:

```text
MLX training/hardening (Python)
  -> hardened.safetensors + manifest.json
Python artifact bridge (training/gbtrain/bridge.py)
  -> f_s5_state_checkpoint_export.v1
gbf compile (Rust)
  -> gbf-codegen import_state_checkpoint
  -> StatefulSubwordProgram
  -> IntStateLoweredModel
  -> gbf-kernel stateful subword backend
  -> gbf-asm cartridge assembly
  -> rom.gb + rom.sym + reports
```

The training checkpoint and tokenizer under `training/artifacts/` are local,
ignored artifacts today. Therefore the checked-in source alone cannot recreate
this exact ROM. Clean-clone reproducibility requires pinning those two inputs in
a content-addressed artifact store (or equivalent) and verifying the hashes in
`build_report.json`. The Rust compiler begins at the bridged checkpoint schema;
this change does not claim that MLX training or the bridge runs in Rust.

For the verified byte-identical local rebuild, the hardened student SHA-256 is
`166d34596df30837a52d7e14a1c6b5cbccb39f1903f9f938350ab131870f57d2`
and the tokenizer SHA-256 is
`ffae9160f720a680d18e2194ab86e44b23732ba0bcec68123cf04f49556443ad`.

The original MLX training run also lacks a complete
command/environment/input-hash record, so the compiler report proves the
bridged-checkpoint-to-ROM build rather than deterministic retraining.
