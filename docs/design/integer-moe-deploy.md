# Integer MoE deployment status

> **Status: implemented capability, not the current production topology.**
> This file used to be a forward-looking gap list. The bridge, loader, integer
> representation, and ROM builder now contain MoE paths, but
> `gbllm-dense-d192-interactive.gb` deliberately uses one dense expert.

## Current production choice

The verified interactive ROM has:

~~~text
d_model=192
d_ff=384
n_blocks=6
state_slots=192
n_experts=1
vocab=1024
~~~

Because `n_experts == 1`, no router runs during training or inference. Every
block executes its one FFN. This gives the current artifact predictable latency,
the simplest parity surface, and a direct hardened-checkpoint-to-ROM proof.

MoE support must not be used to describe this dense artifact.

## Implemented MoE path

For an MLX model with more than one expert:

1. `training/gbtrain/model.py::MoEBlock` evaluates a low-rank fp32 router on
   the raw residual vector and dispatches each token to one expert.
2. `training/gbtrain/export.py` hardens every expert projection and preserves
   the fp32 router tensors.
3. `training/gbtrain/bridge.py` emits
   `f_s8_moe_state_checkpoint_export.v2` with expert-indexed tensor names and
   four router tensors per block.
4. `gbf-bench/src/stateful.rs` validates and loads the MoE schema rather than
   rejecting it.
5. `gbf-kernel/src/state_model_ref.rs` lowers the router and expert weights
   into the integer model representation.
6. `gbf-kernel/src/asm_impl_shell.rs` and
   `gbf-kernel/src/asm_impl_state.rs` contain the runtime dispatch/build path.

The router computes:

~~~text
hidden = input_projection @ residual + input_bias
scores = expert_projection @ hidden + expert_bias
selected = lowest_index_argmax(scores)
~~~

Only the selected expert's ternary FFN is executed for that token. Router
parameters remain fp32 at the bridge boundary; expert and state projections
are ternary with per-row Q8.8 scales.

## Capabilities that are no longer gaps

- The topology carries `n_experts` and router rank.
- The MLX hardening format carries routers and indexed experts.
- The bridge emits a versioned MoE checkpoint schema.
- The Rust loader accepts and verifies that schema.
- The integer model represents dense and top-1 MoE blocks.
- The wide V1024 vocabulary uses paged logits.
- The ROM builder contains on-device subword tokenization and UInt16 token
  feedback.

The old design document's statements that these pieces were absent are
superseded.

## What is not proven by implementation

Code-path existence is weaker than product evidence:

- it does not show that an MoE student beats the current dense student;
- it does not make an MoE checkpoint the source of the named production ROM;
- it does not prove a specific MoE artifact fits the same bank, WRAM, SRAM, and
  latency budgets;
- it does not make the full generic fourteen-stage compiler part of this path;
- the compiler-owned `interactive-subword-dmg` profile currently rejects MoE
  checkpoints and accepts only dense paged-vocabulary recurrent checkpoints;
- it does not establish clean-clone training reproducibility.

Any future MoE release should carry its own model hash, tokenizer hash, ROM hash,
integer-parity evidence, debugger acceptance, quality measurements, ROM/WRAM
budget report, and per-token timing.

## Related truth sources

- MLX model: `training/gbtrain/model.py`
- hardening: `training/gbtrain/export.py`
- schema bridge: `training/gbtrain/bridge.py`
- compiler importer: `gbf-codegen/src/import_state_checkpoint.rs`
- interactive compiler profile: `gbf-codegen/src/compile_state_subword.rs`
- integer lowering: `gbf-kernel/src/state_model_ref.rs`
- ROM generation: `gbf-kernel/src/asm_impl_state.rs`
- current dense product: [repository README](../../README.md)
