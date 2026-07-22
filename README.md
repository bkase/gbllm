# gbllm: a language model that runs on a Game Boy

gbllm trains a small recurrent language model, hardens it for integer
inference, lowers its weights into LR35902 machine code, and packages the model,
tokenizer, sampler, and interactive shell into an 8 MiB Game Boy cartridge.

This README describes the product that actually runs today:

- ROM: `artifacts/builds/gbllm-dense-d192-interactive.gb`
- SHA-256: `230a3842cc44fd8a96393ffaaa75d4954001fc03e9333273393dca9606a75a7b`
- cartridge: 8 MiB MBC5+RAM with 8 KiB cartridge SRAM
- model: dense d192, FFN width 384, six blocks, 192 recurrent state slots
- vocabulary: 1,024 byte-BPE tokens
- generation: 24 tokens, top-k 4, temperature 0.6, deterministic seed `0x5eed`

The ROM above has been rebuilt from the current local hardened model and
tokenizer and compared byte-for-byte with the named artifact. Both files have
the SHA-256 shown above.

## The honest implementation boundary

The production path is:

~~~text
TinyStories bytes
  -> byte-BPE token stream
  -> MLX fp32 teacher
  -> MLX distilled/QAT student
  -> hardened ternary checkpoint
  -> Python checkpoint bridge
  -> gbf compile --profile interactive-subword-dmg
  -> verified Rust checkpoint import
  -> narrow recurrent/subword program + integer lowering
  -> stateful Game Boy backend
  -> gbf-asm
  -> MBC5 Game Boy ROM
~~~

The current interactive ROM does pass through `gbf-cli compile`, using the
compiler-owned `interactive-subword-dmg` profile. That profile is intentionally
narrow: it models the deployed recurrent state, byte-BPE tokenizer, UInt16 token
IDs, prompt shell, and sampler directly. It does **not** pretend that the
generic fourteen-stage compiler design is wired end to end. The same CLI also
has an older, separate `dense-bigram` profile; see
[the compiler gate](docs/experiments/gbf-compile/README.md) for that profile's
exact scope.

There are also two different reproducibility claims:

1. **Hardened model to ROM:** proven locally, byte-for-byte.
2. **Raw training data to the same hardened model:** not yet bit-replayable.
   MLX GPU training is nondeterministic, and the saved training artifact does
   not record every input hash, command-line argument, software version, git
   revision, or dirty-worktree state.

The code is the source of truth for current behavior. Design documents describe
future architecture only when they say so explicitly.

## Why this model

The Game Boy has an 8-bit LR35902 CPU, a small register file, no floating-point
unit, 8 KiB of internal work RAM, and a banked cartridge address space. The
model was selected around those constraints rather than shrunk after training.

| Choice | Production value | Why |
|---|---:|---|
| Residual width | 192 | Wide enough to learn useful features while leaving room for activations, recurrent state, the stack, the UI, and runtime scratch in 8 KiB WRAM. |
| FFN width | 384 | A conventional 2x expansion that adds nonlinear capacity without making generated weight code exceed the 8 MiB cartridge ceiling. |
| Blocks | 6 | Repeated pre-norm residual transformations provide depth while keeping token latency around the measured 22-second range. |
| Recurrent state | 192 slots, four decay bands | Carries context across tokens without a growing KV cache. Decays 0.5, 0.75, 0.875, and 0.9375 give the model short through longer-lived memory traces. |
| FFN topology | Dense, one expert | The exact ROM uses the simpler, deterministic dense path. MoE support exists in the bridge/runtime, but it is not this artifact. |
| Vocabulary | 1,024 byte-BPE tokens | Common byte sequences become one token, so a single expensive model step can emit several characters. Byte fallback keeps every prompt representable. |
| Head | Tied to the embedding | Reusing the embedding avoids training and storing a second V x D matrix. |
| Projection weights | Ternary {-1, 0, +1} plus one Q8.8 scale per output row | Replaces general multiplications with signed accumulation and makes weights suitable for code generation. |
| Activations | Int8-aware QAT on a fixed [-8, 8] grid | Trains the student for the numeric constraints it will encounter on the cartridge. |
| Sampler | top-k 4, T=0.6 | Chooses coherence over diversity for this model size. |

The exact topology is recorded in
`training/artifacts/student_dense_d192/config.json`. The model implementation
is [training/gbtrain/model.py](training/gbtrain/model.py).

## Tokenization and data

The tokenizer in [training/gbtrain/tokenizer.py](training/gbtrain/tokenizer.py)
is a byte-level BPE:

1. IDs 0 through 255 represent raw bytes.
2. Training repeatedly merges frequent adjacent symbols until the vocabulary
   reaches 1,024 entries.
3. Encoding applies the lowest-ranked available merge, breaking ties at the
   leftmost position.
4. Every token retains its decoded byte string, so the Game Boy can render
   multi-byte tokens without a host.

The training launcher loads `artifacts/ds_ts_1024` and
`artifacts/tinystories_bpe_1024.json`. The dataset code in
[training/gbtrain/data.py](training/gbtrain/data.py) forms deterministic,
contiguous truncated-BPTT lanes from the token stream. Contiguous lanes matter:
the recurrent state value continues from one training window to the next even
though gradients are detached at each truncation boundary.

## Teacher and student training

### Teacher

[training/run_teacher.py](training/run_teacher.py) trains a larger dense
full-precision reference model:

- d_model 512;
- d_ff 1,024;
- six blocks;
- 512 recurrent state slots;
- vocabulary 1,024;
- sequence length 256 and 128 lanes;
- 30,000 optimizer steps;
- fp32 MLX execution with QAT disabled.

The teacher is deliberately too large to deploy. Its job is to provide a
smoother distribution over next-token alternatives than one-hot labels alone.

### Student

[training/run_student.py](training/run_student.py) builds the deployable d192
student. The dense artifact overrides `n_experts` to 1 and records step
24,000. Its training objective is:

~~~text
cross entropy
  + distillation_weight * temperature^2 * soft_cross_entropy(teacher, student)
  + router auxiliary loss
~~~

The router term is zero for this dense one-expert artifact. The student uses
AdamW, gradient clipping, truncated BPTT, and carried-but-detached recurrent
state. The loop that composes these losses and advances state is
[training/gbtrain/train.py](training/gbtrain/train.py).

The schedule in
[training/gbtrain/qat_schedule.py](training/gbtrain/qat_schedule.py) makes a
deliberate quality/deployability trade:

1. For the first 40% of steps, weight and activation fake quantization are off.
   The student first learns the teacher's broad behavior.
2. At step 9,600 of the 24,000-step run, both switches move directly to hard
   quantization.
3. Projection weights become ternary through a straight-through estimator.
4. Per-row scales are snapped to Q8.8, and selected activations are snapped to
   an Int8 grid over [-8, 8].
5. The learning rate decays through the hard phase while the distillation
   weight rises from 0.5 to 0.65.

Hard QAT is crucial. Post-training quantization would ask a float model to
survive a new arithmetic system it never saw. Here the deployed numeric errors
are present during the final 60% of optimization, so the model can adapt its
representations around them.

### Hardening

[training/gbtrain/export.py](training/gbtrain/export.py) converts latent
training parameters into `gbllm_student_hardened.v1`:

- every state and FFN projection becomes an Int8 tensor containing only
  -1, 0, or +1;
- every output row receives a UInt16 Q8.8 scale;
- the shared embedding/head remains fp32 in the hardened artifact;
- the four recurrent decay values are recorded as Q8.8 raws
  128, 192, 224, and 240.

The exporter reloads its own result and reevaluates it, checking that the saved
deployable form matches the student's final hard-QAT behavior.

## From hardened model to ROM

### 1. Bridge the Python artifact

[training/gbtrain/bridge.py](training/gbtrain/bridge.py) is a NumPy-only bridge.
For a dense student it emits `f_s5_state_checkpoint_export.v1`: a JSON
manifest plus 30 little-endian, row-major tensor files, each with a SHA-256.

~~~bash
mkdir -p /tmp/gbllm-interactive-build
PYTHONPATH=training training/.venv/bin/python -c \
  "from gbtrain.bridge import bridge_hardened_export; \
bridge_hardened_export( \
  'training/artifacts/student_dense_d192', \
  '/tmp/gbllm-interactive-build/ckpt')"
~~~

The bridge changes packaging, not learned values. The compiler-owned importer
in
[gbf-codegen/src/import_state_checkpoint.rs](gbf-codegen/src/import_state_checkpoint.rs)
checks the schema, topology, shapes, dtypes, and every tensor hash before
constructing a `StateCheckpoint`. The former benchmark loader is now only a
compatibility re-export of this importer.

### 2. Lower to Game Boy integers

`IntStateLoweredModel::lower` in
[gbf-kernel/src/state_model_ref.rs](gbf-kernel/src/state_model_ref.rs):

- lowers embeddings to the residual representation;
- derives an Int8 tied-head table from the same embedding;
- expands per-row Q8.8 scales;
- proves accumulator widths and selects i24 where i16 is insufficient;
- installs activation, GELU, sampling, and dequantization lookup tables;
- preserves 16-bit token IDs for the 1,024-entry vocabulary.

Vocabulary 1,024 is represented as a paged head. For this exact production
shape, all 1,024 three-byte logits fit in cartridge SRAM, so the builder selects
`SramFull`.

### 3. Turn weights into code and assemble the cartridge

The production entry point is `gbf compile`, whose interactive orchestration
lives in
[gbf-codegen/src/compile_state_subword.rs](gbf-codegen/src/compile_state_subword.rs):

~~~bash
cargo run --release -p gbf-cli -- compile \
  --profile interactive-subword-dmg \
  --checkpoint-export /tmp/gbllm-interactive-build/ckpt \
  --tokenizer training/artifacts/tinystories_bpe_1024.json \
  --tokens 24 \
  --top-k 4 \
  --temperature 0.6 \
  --rng-seed 0x5eed \
  --out /tmp/gbllm-interactive-build/out

cmp /tmp/gbllm-interactive-build/out/rom.gb \
  artifacts/builds/gbllm-dense-d192-interactive.gb
~~~

The compiler imports and hashes the checkpoint and tokenizer, rejects anything
other than a dense recurrent checkpoint with a paged vocabulary, constructs a
truthful `StatefulSubwordProgram`, invokes `IntStateLoweredModel::lower`, and
writes `rom.gb`, `rom.sym`, `build_report.json`, and `compile_request.json`.
The benchmark emitter remains only as a compatibility wrapper.

The stateful backend lives in
[gbf-kernel/src/asm_impl_shell.rs](gbf-kernel/src/asm_impl_shell.rs) and
[gbf-kernel/src/asm_impl_state.rs](gbf-kernel/src/asm_impl_state.rs).
Its V3 lowering specializes ternary matrix rows into straight-line LR35902
instructions:

- +1 emits an addition;
- -1 emits a subtraction;
- 0 emits no operation;
- general weight multiplications disappear from the hottest FFN path.

`gbf-asm` then resolves labels and banked relocations, emits the Nintendo
header and computes cartridge checksums. The exact build occupies 408 banks.
Bank 0 contains the driver and shell; switched banks contain generated weight
code, embeddings, tied-head data, scales, lookup tables, tokenizer merges,
token byte strings, font/UI assets, and other constants.

## What executes on the Game Boy

After boot, the cartridge is self-contained. The emulator/debugger is not an
inference coprocessor.

1. The joypad drives a 4x19 on-screen keyboard. A types, B deletes, and START
   submits up to 20 ASCII bytes.
2. The ROM runs the same greedy byte-BPE merge rules as the Python tokenizer.
3. It zeroes the 192-slot recurrent state.
4. Each prompt token is embedded and prefills the recurrent model.
5. For each token, the model executes the LinearState update and six residual
   FFN blocks, normalizes the result, and evaluates the tied V1024 head.
6. The 1,024 i24 logits are retained in 8 KiB cartridge SRAM.
7. The ROM samples top-k 4 at temperature 0.6 using its built-in XorShift16
   state, initially `0x5eed`.
8. The sampled UInt16 token ID is fed back through the embedding, while its byte
   string is painted into the transcript.
9. After 24 generated tokens the shell returns to idle.

The exact BPE encoder, recurrent prefill, generated-token feedback, and shell
loop are emitted by
[gbf-kernel/src/asm_impl_shell.rs](gbf-kernel/src/asm_impl_shell.rs).

## How intelligence survives an 8 MiB cartridge

The cartridge does not contain training text or call a remote model. It
contains a compressed learned function.

- **Distillation transfers behavior.** The d512 teacher exposes relationships
  among many possible next tokens. The d192 student learns those relationships,
  not just the single recorded target.
- **Parameters store statistical structure.** About 1.16 million trained
  parameters encode reusable features for spelling, word fragments, syntax,
  names, story patterns, and local semantic associations.
- **Ternary does not mean binary behavior.** Projection signs are ternary, but
  each row has a learned scale, activations carry multiple bits, recurrent state
  is i24, nonlinearities remain, and six residual blocks compose these pieces.
- **Recurrent memory replaces a KV cache.** The 192 state slots summarize the
  prior prompt and generated tokens. Four decay bands let different state
  regions remember at different time scales with constant memory per token.
- **BPE buys more language per expensive step.** A token can represent a common
  multi-byte fragment, so the model's 1,024-way prediction carries more text
  than a character-only head.
- **QAT spends capacity where the final machine needs it.** The student learns
  under the same ternary/Int8 pressure that deployment imposes.
- **Code generation exchanges ROM for CPU capability.** The 8-bit CPU cannot
  perform modern tensor operations efficiently, so compilation turns static
  weights into specialized additions and subtractions. The full 8 MiB is used
  as an executable representation of the network.

This is still a very small language model. It produces recognizable,
context-sensitive English-like story continuations, but it does not have the
knowledge, long-horizon coherence, or instruction-following ability of modern
large models. The meaningful result is that genuine learned autoregressive
inference—tokenization, recurrent context, nonlinear layers, a 1,024-token
head, sampling, and feedback—runs entirely on Game Boy hardware.

## Verification

Run the independent scripted acceptance:

~~~bash
scripts/review/bd-36akp/verify.sh
~~~

The script uses `gbf-debug` to send JOYP press/release frames only. It verifies
that the cartridge tokenizes `Once upon a time` to
`[435, 443, 258, 402]`, prefills those IDs on-device, produces the pinned
24-token sequence, renders the expected transcript, stays below 30 seconds per
generated token, and returns to idle. The evidence and measured timings are in
[docs/experiments/interactive-subword](docs/experiments/interactive-subword/README.md).

Important input identities for the byte-identical local rebuild are:

| Input/output | SHA-256 |
|---|---|
| Hardened student | `166d34596df30837a52d7e14a1c6b5cbccb39f1903f9f938350ab131870f57d2` |
| Hardened manifest | `3a300b883887318ed10844f81d919062ecdd1a1935f4027c116a97330c843642` |
| Tokenizer | `ffae9160f720a680d18e2194ab86e44b23732ba0bcec68123cf04f49556443ad` |
| Bridged manifest | `f52e89930a5ea38704b944436a9e8cf4991d60cb4f53242bb614524cd72c2435` |
| Final ROM | `230a3842cc44fd8a96393ffaaa75d4954001fc03e9333273393dca9606a75a7b` |

## Reproducibility work still needed

`training/artifacts/` is ignored, and the exact ROM is currently an untracked
local artifact. A clean clone therefore cannot reproduce this cartridge without
separately obtaining the pinned hardened student and tokenizer.

The compiler now writes a build report beside every output ROM. It records the
verified bridged-manifest and tokenizer hashes, topology, sampler, storage,
lowering, ROM hash, compiler version, and an explicit list of real versus
unwired stages. What remains is upstream provenance: training manifests need
the actual argv and hashes for the dataset, tokenizer, teacher, source tree,
and runtime environment; release inputs need content-addressed storage; and the
compiler git revision should be mandatory rather than optional. Until those
checks exist, documentation must say “verified local hardened-model-to-ROM
rebuild” rather than claiming a hermetic raw-data-to-ROM replay.
