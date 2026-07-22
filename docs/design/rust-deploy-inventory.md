# Rust deployment inventory

> **Status: current code map.** This inventory separates the path used by the
> interactive d192/V1024 ROM from compiler components that exist elsewhere in
> the workspace.

## Production interactive path

The exact entry point for
`artifacts/builds/gbllm-dense-d192-interactive.gb` is `gbf compile --profile
interactive-subword-dmg`. Its orchestration is owned by
`gbf-codegen/src/compile_state_subword.rs`; the old benchmark emitter is a
compatibility wrapper around the same compiler function.

Its dependency path is:

~~~text
bridged checkpoint
  -> gbf-codegen::import_state_checkpoint
  -> gbf-data::bpe::BpeModel
  -> gbf-codegen::compile_state_subword::StatefulSubwordProgram
  -> gbf-kernel::state_model_ref::IntStateLoweredModel::lower
  -> gbf-kernel::asm_impl_shell::build_state_subword_shell_rom_with_seed
  -> gbf-kernel::asm_impl_state
  -> gbf-asm::assemble_rom
  -> rom.gb + rom.sym + build_report.json + compile_request.json
~~~

## Crate and module ownership

| Component | Current responsibility |
|---|---|
| `gbf-data/src/bpe.rs` | Parse the byte-BPE JSON, expose merge ranks and decoded bytes for every token ID. |
| `gbf-codegen/src/import_state_checkpoint.rs` | Import dense f_s5 v1 and MoE f_s8 v2 state checkpoints; verify manifest/tensor hashes, topology, dtypes, and shapes. |
| `gbf-codegen/src/compile_state_subword.rs` | Own the narrow recurrent/subword program, interactive compile options, compiler orchestration, stage-coverage facts, and output packet. |
| `gbf-kernel/src/state_model_ref.rs` | Own stateful float/integer reference structures, paged-logit choice, integer lowering, accumulator proofs, and host parity evaluation. |
| `gbf-kernel/src/asm_impl_shell.rs` | Emit keyboard UI, exact BPE prompt encoding, prefill, sampling, UInt16 feedback, transcript rendering, and the interactive generation loop. |
| `gbf-kernel/src/asm_impl_state.rs` | Plan WRAM/SRAM/ROM layout and emit state, FFN, head, sampler, and generated weight-code routines. |
| `gbf-kernel/src/decode.rs` | Define sampler configuration and fixed-point temperature handling. |
| `gbf-asm` | Model LR35902 instructions/sections/symbols, lay out banks, resolve relocations, and write the cartridge header/checksums. |
| `gbf-emu` | Provide cycle-aware Game Boy execution for regression and acceptance harnesses. |
| `gbf-debug` | Drive scripted ROM sessions using symbols, breakpoints, JOYP input, and memory inspection. |

## Model lowering used by the ROM

The production stateful lowerer:

- converts embeddings into the residual representation;
- derives the quantized tied-head view from the same embedding;
- carries ternary projections with Q8.8 row scales;
- validates state/input dimensions and numeric bounds;
- selects i16 or i24 accumulator paths from structural proofs;
- supports dense and top-1 MoE block representations;
- chooses paged logits for V1024;
- retains full UInt16 token identity through sampling and feedback.

V3 weights-as-code is selected by the interactive builder. Nonzero ternary
weights become straight-line add/subtract sequences; zeros are omitted.

## Cartridge layout used by the production shape

The d192/ff384/six-block/V1024 dense shape selects:

- 8 MiB MBC5 ROM;
- 8 KiB cartridge SRAM;
- `SramFull` logit storage;
- 408 occupied banks in the verified build;
- bank 0 for the driver and interactive shell;
- switched banks for generated weight code, parameter tables, tokenizer data,
  and UI/font assets.

## Compiler profiles and the generic-pipeline boundary

`gbf-cli compile` currently exposes two real but deliberately narrow profiles:

- `interactive-subword-dmg` enters
  `gbf-codegen/src/compile_state_subword.rs`, accepts the production dense
  recurrent checkpoint plus byte-BPE tokenizer, and builds the current
  interactive V1024 cartridge;
- `dense-bigram` enters `gbf-codegen/src/compile.rs`, imports
  `f_s6_dense_checkpoint_export.v1`, and builds the older stateless d64/V256
  compiler-gate cartridge.

Neither path fabricates the full generic fourteen-stage pipeline. The
interactive profile reports generic Stage 0 `ArtifactView`, `GbInferIR`,
window/overlay/arena scheduling, and generic stage-cache products as unwired.
The dense-bigram profile likewise uses a narrow `DenseBigramProgram`; its
`lower_infer.rs` explicitly says that IR is not the full `GbInferIR`.

The workspace may contain implemented types or isolated tests for generic
stages. Their presence does not put them in either production call graph.

## Acceptance path

`scripts/review/bd-36akp/verify.sh` drives
`scripts/review/bd-36akp/interactive-acceptance.js` through `gbf-debug`.
The script supplies only JOYP input, then observes symbols and device memory to
check on-cartridge BPE, recurrent prefill, generation cadence, sampled IDs,
rendered bytes, and return to idle.

See the [repository README](../../README.md) for the complete current-product
explanation and reproducible local build command.
