# Integer MoE deploy path — host evaluator + ROM (bd-uzxic, bd-3vr6s)

Status: design (2026-07-09). Prereqs DONE: V2 dispatch lowering (bd-1tuql),
export bridge `f_s8_moe_state_checkpoint_export.v2` (bd-3mzda), MLX↔Rust **f32**
MoE parity (bd-2lk86, `gbf-bench/src/moe_parity.rs`, max_abs_diff 9.5e-7). This
doc plans the remaining layers that turn the trained subword MoE d192×8 student
into a byte-exact ROM.

## What already works
- **f32 seam proven.** The bridge writes the deployed low-rank router
  (`router.rs` `Top1RouterQat`: `hidden = Win·x + bin`; `raw = Wout·hidden +
  bout`; `argmax(raw)`, lowest-index tiebreak; on the **raw pre-norm residual**;
  f32) plus per-expert ternary up/down and a tied V=1024 head. A self-contained
  Rust f32 forward reproduces the MLX student's logits to the rounding floor.
- **Dense integer semantics** (`state_model_ref.rs`, `state-int-semantics.v2`)
  are byte-exact and ROM-buildable via V2 dispatch for a single FFN per block.

## Gap to a deployable MoE ROM
Three orthogonal pieces, in dependency order.

### 1. Host integer MoE evaluator (the deterministic reference)
`state_model_ref.rs` today: `StateTopology` has no `n_experts`; the loader
(`gbf-bench/src/stateful.rs`) hard-rejects `moe:true`. Plan:

- **Schema/struct.** Add `n_experts` to `StateTopology` (default 1 = dense,
  back-compat). Replace each block's single `(up,down)` with a `BlockFfn` enum:
  `Dense { up, down }` | `Moe { router: LowRankRouter, experts: Vec<(up,down)> }`.
  `LowRankRouter` carries the four f32 tensors + `rank`.
- **Loader.** New path in `load_state_checkpoint` for
  `f_s8_moe_state_checkpoint_export.v2`: parse `layers[]`, load the four router
  f32 tensors per block by the bridge's names
  (`block{b}_router_{input_projection,input_bias,expert_projection,expert_bias}`)
  and `block{b}_expert{e}_{up,down}.{ternary,scales}`.
- **Forward.** At each MoE block, the integer residual is i24 Q19.5. Dispatch:
  1. **Dequantize** the current residual `x_i24 → x_f32 = x_i24 / 32` (Q19.5).
  2. Run the **f32 low-rank router** on `x_f32` → `raw[e]` → `argmax`
     (lowest-index tiebreak). *The router output is used ONLY to pick the expert
     index — it never re-enters the integer stream.*
  3. Run the selected expert's up/down through the **existing byte-exact integer
     FFN path** (`int_norm_quant24 → i16 up matvec → gelu LUT → i16/i24 down
     matvec → i24 wrap add`). Zero new integer math — experts reuse the dense
     block kernel verbatim, so byte-exactness is inherited.
- **Determinism caveat (must gate).** The router argmax is over f32 sums. Near
  ties, libm/order differences could flip the expert → different output. On host
  this matches MLX (both f32). For the **ROM** (no FPU), the router must become
  fixed-point. Decision: compute the router in **Q8.8/Q16.16 integer** on-device
  and *also* in the host reference, so host==ROM by construction; validate the
  fixed-point router argmax agrees with the f32 router on the real student across
  the eval set (log any divergent-token count; require 0 on the deployed model,
  else widen the router fixed-point width). Track margin `raw[top1]-raw[top2]`.
- **Tests.** (a) `n_experts=1` MoE path == the existing dense path byte-for-byte
  (regression against current ROM gates). (b) Integer MoE forward on the real
  student vs an integer golden dumped from a host reference. (c) Fixed-point vs
  f32 router argmax agreement on the real student (0 divergences required).

### 2. V=1024 logit paging (bd-3vr6s)
`StateTopology::validate` caps `vocab ≤ 85` (3-byte i24 logits in one 256-byte
page). V=1024 needs 1024·3 = 3072 B of logits — impossible in one page and it
won't co-reside in 8 KiB WRAM with activations anyway. Plan (host + ROM):
- Compute logits in **pages of ≤85 ids**: 13 pages for V=1024. Each page streams
  the tied-head i8 rows for its id range, accumulates the i32 dot products,
  keeps a **running top-1 (or top-k) argmax** across pages, and discards the page
  buffer. WRAM cost is one page + the running max, not the full logit vector.
- Sampling: for greedy, running argmax suffices. For temperature/top-k, keep a
  running top-k heap (k≤~40) across pages, then softmax over just those k on the
  final page. The host reference must mirror this exact two-pass/heap logic.
- Relax `validate` to allow `vocab` up to the paged ceiling when a
  `LogitPaging` capability is set; keep the 85 single-page cap as the default so
  existing dense ROMs are unaffected.

### 3. On-device MoE expert bank dispatch (bd-uzxic, ROM builder)
V2 dispatch already packs each ternary matrix as base-81 data behind a shared
bank-0 handler. For MoE:
- Lay out each block's `n_experts` expert weight streams in ROM banks.
- After the fixed-point router argmax picks `e`, **MBC5 bank-switch** to expert
  `e`'s up/down streams and run the same V2 matvec handler. One expert's banks
  are resident per token; the other experts cost ROM, not WRAM or cycles.
- The router's four f32 tensors are tiny (rank≈2..8) — store as fixed-point in
  bank 0 / a resident page.
- Cycle budget: one expert per block ≈ the dense per-token cost, so the
  measured ~45 s/token d192 dense number is the right ballpark; experts add ROM
  banks, not per-token MACs. Re-measure on the real student.

## Build order
1. Host integer MoE evaluator + `n_experts=1`==dense regression + real-student
   integer golden (unblocks everything; no ROM yet).
2. V=1024 logit paging in the host reference + sampler (greedy + top-k heap).
3. Fixed-point router + host/ROM argmax-agreement gate on the real student.
4. ROM builder: expert bank dispatch + paged-logit epilogue, byte-exact vs the
   host integer evaluator on the real student (mirror the existing d192-real
   parity gate).
5. Subword deploy surface: on-device token→bytes render table (id_bytes) so a
   token paints multiple chars; greedy BPE encode of the typed prompt. Gate
   against the Python↔Rust conformance vectors (`gbtrain/conformance.py`).

## Non-goals / open
- Training-replay determinism stays deferred (fp GPU training is
  non-deterministic by design; the deployed integer ROM is the determinism
  contract).
- Whether hard one-hot routing (no task-gradient to the router) beats a dense
  d192 FFN of equal active-param cost is an empirical question answered by the
  finished student's eval; if MoE underperforms, fall back to prob-scaled
  (Switch) routing which needs a ROM scalar-multiply of the expert delta by the
  top-1 routing probability (small addition to step 3).
