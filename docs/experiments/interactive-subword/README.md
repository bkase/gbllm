# Interactive subword cartridge

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

```bash
cargo run --release -p gbf-bench --bin emit-interactive-subword-rom -- \
  /path/to/bridged-dense-student \
  artifacts/builds/gbllm-dense-d192-interactive.gb \
  24
```

The bridged directory contains `ckpt/` plus
`tokenizer/gbllm_bpe.v2.json`. The emitter also writes an adjacent `.sym` file
for the debugger acceptance script.
