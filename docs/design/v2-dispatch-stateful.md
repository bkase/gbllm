# V2 dispatch lowering in the stateful/model ROM builder (bd-1tuql)

> **Status: historical design note for the optional V2 dispatch path.** The
> cared-for dense d192/V1024 ROM selects V3 weights-as-code, as recorded by the
> compiler build report and the [repository README](../../README.md). Statements
> below about what was absent or planned describe the state when this note was
> written, not the current implementation inventory.

## Why

The deployable ROM builder (`gbf-kernel/src/asm_impl_state.rs`,
`asm_impl_model.rs`) emits **only V3 weights-as-code**: each weight becomes
straight-line `add`/`sub` machine code. Real d192 is ~6.07 MiB / 405 banks and
nearly dense (0.85% zeros) — already 79% of the MBC5 512-bank / 8 MiB ceiling.
A dense d256 needs ~10.8 MiB of weight code → **does not fit at all**.

V2 dispatch stores each weight as a packed base-81 index (~0.25 B/weight, ~26×
denser than V3's ~6.6 B/weight) walked by a **shared** handler. Same 8 MiB then
holds a much larger model (and enough experts for MoE). Latency budget is now
up to **120 s/char** (~350M M-cycles/token), so the ~2× cycle cost of dispatch
vs straight-line code is affordable. This is the enabler for every
"bigger model" / MoE item on the roadmap.

## Hard invariant

The ROM must stay **byte-exact** against the canonical integer evaluator
(`model_ref` / `state_model_ref`) so it remains correct on real hardware.
V3 stays the default; V2 is opt-in behind a `WeightLowering` enum until every
existing byte-exact gate passes under V2.

## Key correctness insight

V2 reproduces the **same integer math** as V3 per row/segment:
`acc = bias + Σ (±act_j)`, mod 2¹⁶, where `bias = -128·Σ row`. V3 emits this as
code; V2 emits the weights as data and the add/sub as a shared handler. The
result written to WRAM `l.acc` is identical bit-for-bit. Therefore **all
epilogues, scale application (Q8.8), the i24 down-delta combine, norms, GELU,
head, sampler, WRAM layout, and the forward-pass call order are untouched.**
Only the per-matvec accumulation emission changes.

## Stream format (per matvec, reusing `spec::base81_stream` conventions)

`bias_0 (i16 LE) | row_0 base-81 bytes | ROW_END(81) | bias_1 | ... |
bias_last | row_last | MATRIX_END(82)`. Base-81 byte = `t0 + 3·t1 + 9·t2 +
27·t3` over the four trits (`00`→0, `01`→+1, `10`→−1). Weights become **data**
laid contiguously across banks (not one-matvec-per-bank like V3 chunks).

Wide (i24 down) matvec: segment each row into 192-col segments (the existing
`WIDE_SEGMENT_COLS` bound where a segment's partial fits i16), frame each
segment with `bias | seg bytes | SEG_END`, and reuse the exact
`encode_row_wide` i24 byte-serial combine in the epilogue.

## Shared dispatch routine (bank-0 resident, ~2.7 KB)

Registers (mirrors the bake-off `build_v2_dispatch`, extended for banking):
- `DE` = i16 accumulator (single signed acc; add/sub directly — matches V3
  `P−N` mod 2¹⁶).
- `SP` = activation pointer, re-seeded to `l.act` at each row/segment start
  (interrupts disabled during the walk, as the V3 chunks already require).
- stream pointer walks packed bytes at `0x4000–0x7FFF`; **bank-crossing**: after
  advancing, if the pointer hi byte reaches `0x80`, write the next 9-bit bank to
  `MBC5_ROMB0/ROMB1` and reset the pointer to `0x4000`. Bank counter + stream
  pointer live in the fixed scratch page the handlers never touch (same page
  `chunk_run` uses: `CHUNK_BANK`, and new `WSTREAM` scratch).
- dispatch table: 83 entries (81 patterns + `ROW_END`/`SEG_END` +
  `MATRIX_END`) at a fixed 256-aligned bank-0 address; each handler decodes its
  4-trit pattern into up to four `pop`+add/sub of the popped activation pair,
  then re-dispatches. `ROW_END` stores DE→`*out_ptr`, bumps `out_ptr` by 2,
  re-seeds bias+SP; `SEG_END` runs the i24 combine; `MATRIX_END` returns.

Bank-crossing is the only genuinely new machinery vs the bake-off V2; it mirrors
`chunk_run`'s MBC5 9-bit bank walk (`asm_impl_state.rs:2746`) at byte
granularity.

## Integration points

- `StateRomPlan.per_matvec_chunks: Vec<Vec<Vec<u8>>>` (built at
  `asm_impl_state.rs:2611` by `build_matvec_chunks_at` / `_wide`). Add a
  parallel V2 representation: `per_matvec_stream: Vec<Vec<u8>>` (one packed
  stream per matvec) plus a `WeightLowering` on the plan.
- Forward pass (`emit_state_forward_body`, `asm_impl_state.rs:2783`): replace
  each `emit_call_chunks(...)` with, under V2, a `set_wstream(bank, len)` +
  `call "matvec_v2"`. Bank bookkeeping mirrors the existing `next_bank` walk;
  stream bytes are packed into banks by a new allocator in `plan_state_rom`.
- Bank-count accounting (`asm_impl_state.rs:2688`, the `> 512` guard): V2 weight
  banks = `ceil(total_stream_bytes / 16384)` instead of one-chunk-per-bank.
- Shared routine emitted once in `emit_state_routines_and_tables`
  (`asm_impl_state.rs:2878`) alongside `emit_chunk_run`.

## Incremental plan (each step gated byte-exact before the next)

1. `WeightLowering{ V3, V2Dispatch }` on the plan; V3 path unchanged; all
   current gates green (no behavior change).
2. V2 for the **i16** matvecs (state_in, up, i16-down). Emit shared routine +
   packed streams + per-matvec bank walk with cross-bank streaming. Gate:
   `d192-readiness` synthetic one-token + 64-token byte-exact under V2; d64
   arm-B (`stateful-rom`) still byte-exact under V3.
3. V2 for the **wide i24 down** path (segment framing + reuse i24 combine).
   Gate: full synthetic d192 (i24 down) byte-exact under V2.
4. Bank-packing allocator + `bank_count` accounting; confirm a **d256** lowered
   model builds under V2 within 512 banks / 8 MiB. Add a d256 synthetic
   byte-exact gate.
5. Wire `WeightLowering::V2Dispatch` selection (default stays V3; opt-in via the
   export/bring-up path), re-run `d192-real` under V2 for byte-exact parity +
   cycle accounting, run all hook gates.

## Test commands (byte-exact harness = host integer evaluator vs emulated ROM)

- Synthetic d192: `cargo run --release -p gbf-bench --bin d192-readiness`
- Arm-B d64 regression: `cargo run --release -p gbf-bench --bin stateful-rom`
- Real checkpoint: `cargo run --release -p gbf-bench --bin d192-real`
- Hook gates: `cargo fmt --check --all`; workspace clippy `-D warnings`;
  workspace `--lib --bins`; `gbf-test` qat suites; `runtime_asm_conformance`.

## Non-goals (this bead)

Training the bigger model, MoE routing on-device, and sparsity tuning are
separate beads. This bead only makes a >d192 / MoE-capacity model **fit and run
byte-exactly** on the cart.
