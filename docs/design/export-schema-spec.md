# Model export and bridge schemas

> **Status: current implementation inventory.** Schema truth lives in
> `training/gbtrain/export.py`, `training/gbtrain/bridge.py`, and
> `gbf-codegen/src/import_state_checkpoint.rs`. The production interactive
> profile continues through `gbf-codegen/src/compile_state_subword.rs`,
> `gbf-kernel`, and `gbf-asm`. This document records what those producers and
> consumers accept today.

## There are two export boundaries

The MLX trainer and Rust runtime do not read the same physical artifact.

### 1. Hardened MLX artifact

`training/gbtrain/export.py::export_hardened` writes:

~~~text
<student>/
  config.json
  weights.safetensors
  hardened.safetensors
  manifest.json
~~~

The hardened manifest format is `gbllm_student_hardened.v1`. It contains:

- topology: d_model, d_ff, block count, state slots, expert count, vocabulary,
  and optional router rank;
- one fp32 embedding used for both token lookup and the tied output head;
- Q8.8 recurrent decay raws;
- hardened projection tensors, each with Int8 {-1, 0, +1} weights and UInt16
  Q8.8 scales.

MoE artifacts additionally contain four fp32 low-rank router tensors per block
and expert-indexed up/down projections.

The exporter reloads `hardened.safetensors` into the MLX model and reevaluates
it. That round trip is the trainer-side proof that serialization preserves hard
QAT behavior.

### 2. Rust-consumable bridged checkpoint

`training/gbtrain/bridge.py::bridge_hardened_export` reads the hardened
artifact and writes:

~~~text
<bridged>/ckpt/
  manifest.json
  tensors/*.bin
~~~

Every tensor entry records a semantic name, role, dtype, shape, row-major
layout, relative file, and SHA-256. Files are little-endian. The bridge is
NumPy/stdlib-only and does not quantize a new float checkpoint: it repackages
values already hardened by the trainer.

## Implemented bridge schemas

| Topology | Schema | Layer representation |
|---|---|---|
| Dense, one expert | `f_s5_state_checkpoint_export.v1` | One ternary up/down pair per block |
| MoE, more than one expert | `f_s8_moe_state_checkpoint_export.v2` | Low-rank fp32 router plus indexed ternary experts |

Both schemas use:

- fp32 embedding with a tied head;
- ternary state-input and state-output projections;
- per-output-row Q8.8 scales;
- per-slot Q8.8 multi-timescale decay;
- explicit sequence-state topology;
- tensor-level SHA-256 verification.

There is no production `f_s5_state_checkpoint_export.v2` untied-head path.
The current V1024 cartridge does not require one: it keeps the head tied and
uses paged logits plus cartridge SRAM.

## Dense tensor naming

The bridge maps hardened MLX keys to Rust checkpoint names:

| Hardened key | Rust base |
|---|---|
| `embedding` | `embedding` |
| `state_block.state_in` | `state_input_to_state` |
| `state_block.state_out` | `state_state_to_output` |
| `blocks.B.experts.0.up` | `blockB_up` |
| `blocks.B.experts.0.down` | `blockB_down` |

Each ternary base produces `BASE.ternary` and `BASE.scales` entries whose
files end in `.ternary.i8.bin` and `.scales.q8_8_u16le.bin`.

## MoE tensor naming

For MoE, block B and expert E use:

- `blockB_expertE_up.ternary` and `.scales`;
- `blockB_expertE_down.ternary` and `.scales`;
- `blockB_router_input_projection`;
- `blockB_router_input_bias`;
- `blockB_router_expert_projection`;
- `blockB_router_expert_bias`.

The router semantics are:

~~~text
hidden = input_projection @ raw_residual + input_bias
scores = expert_projection @ hidden + expert_bias
expert = lowest_index_argmax(scores)
~~~

Router parameters stay fp32 in the bridged format. Expert and state
projections are ternary with Q8.8 row scales.

## Importer behavior

`gbf-codegen::import_state_checkpoint`:

1. accepts dense f_s5 v1 or MoE f_s8 v2;
2. validates the manifest topology and schema/topology agreement;
3. verifies every referenced tensor's SHA-256 before parsing;
4. validates shapes, element counts, dtypes, and ternary values;
5. builds the corresponding dense or MoE state checkpoint;
6. selects single-page logits for small vocabularies and `LogitPaging::Paged`
   when the vocabulary exceeds the single-page limit.

The production V1024 checkpoint therefore imports through the paged path.
`gbf-bench::stateful::load_state_checkpoint` is now a compatibility re-export
of this compiler-owned importer. The former claim that the loader rejects MoE
or caps deployment at 85 vocabulary entries is obsolete.

For the cared-for dense ROM, `gbf compile --profile
interactive-subword-dmg` passes the imported checkpoint and tokenizer through
the narrow `StatefulSubwordProgram`, `IntStateLoweredModel`, the stateful V3
backend, and `gbf-asm`. This is a real compiler-owned product path, but it does
not fabricate the still-unwired generic fourteen-stage products.

## Current provenance limitation

The hardened and bridged manifests are sufficient to validate tensor contents,
but they are not a complete release provenance record. In particular, the
production chain does not fail closed on missing:

- git revision and dirty state;
- actual training argv and full effective TrainConfig;
- teacher, dataset, and tokenizer hashes;
- Python, MLX, Rust, and target versions;
- a hash linking the hardened source artifact to the bridged manifest;
- a mandatory compiler git revision rather than the currently optional
  `GBF_COMPILER_GIT_REVISION`.

The interactive compiler now emits `build_report.json` and
`compile_request.json`, including checkpoint-manifest, tokenizer, and ROM
hashes plus topology, sampler, storage, and honest stage-coverage facts. Those
reports close the build-side gap; they cannot reconstruct provenance that the
training and bridge inputs never recorded.

Documentation must therefore distinguish a verified local
hardened-checkpoint-to-ROM rebuild from a hermetic clean-clone training replay.
The repository [README](../../README.md) records the exact current boundary.
