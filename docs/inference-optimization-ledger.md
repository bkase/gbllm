# Game Boy inference speed — optimization ledger

> **Status: historical optimization record for
> `artifacts/builds/gbllm-shell-d192.gb`.** Its measurements and head shape are
> properties of that legacy cartridge, not the current V1024 interactive ROM.
> The current product is documented in [the repository README](../README.md).

Running log of every speed optimization applied to the then-deployed **dense
d192 state model** running on DMG
(`artifacts/builds/gbllm-shell-d192.gb`). One entry
per landed change: what it was, *why it worked*, the measured effect, and how it
was proven safe.

## Ground rules (invariants held by every entry)

- **Numerics never change.** Every entry is byte-for-byte identical output vs the
  host integer evaluator. The generated token stream, all WRAM dumps, and the
  logits are unchanged — only the cycle cost moves.
- **Proof of safety:** `cargo test --release -p gbf-bench --test d192_generation_regression`
  (byte-exact one-token + 12-token on-device generation on the real d192, ~7s)
  must pass, and the arm-B regression (`state_arm_b_regression`) must stay green
  since the state kernel is shared across topologies.
- **Measurement:** `cargo run --release -p gbf-bench --bin cycle-profile` single-
  steps one token in the cycle-accurate emulator and attributes M-cycles by PC
  region (`>=0x4000` = switched weight-code banks / V3 matvec; `<0x4000` = fixed
  driver bank) and by named driver routine. DMG runs at 1,048,576 M-cycles/sec.

## The key insight that drives this work

The kernel-bakeoff established V3 "weights-as-code" as the fastest matvec and the
project treated ~21.7 s/char as near the floor. The profiler showed otherwise:
**per token, the V3 matvec is only ~31% of cycles; the driver bank is ~69%.**
The matvec was already tuned; the driver never was. So the wins are in the
driver: the interpreted state out-projection, the RMS norm, the tied head, and
the software multiplies.

## Progress

| # | change | s/char | Δ vs prev | cum. vs 21.695 |
|---|--------|-------:|----------:|---------------:|
| 0 | baseline (committed d192-real) | 21.695 | — | — |
| 1 | out-projection: register-resident cursors | 18.707 | −13.8% | −13.8% |
| 2 | out-projection: HRAM accumulator + `add a,(hl)` | 18.214 | −2.6% | −16.0% |
| 3 | `mul16x8`: zero-multiplier early-out | 17.782 | −2.4% | −18.0% |

---

## Entry 1 — state out-projection matvec: register-resident cursors

**Commit:** `2a966c3` · **File:** `gbf-kernel/src/asm_impl_state.rs` (`emit_state_out_matvec`)
**Effect:** 21.695 → 18.707 s/char (−13.8%). `smv_col` 18.5% → 7.3% of the token.

**What it was.** The state out-projection (ternary weights × carried i24 state,
~36,864 MACs) was the single biggest driver cost (~25% of the token) and the one
matvec *not* done as weights-as-code — because it multiplies against **i24 state
activations**, not i8, which won't fit the V3 pop-based dispatch. So it was a
memory-to-memory interpreter: it kept the weight cursor and the state cursor as
2-byte pointers **in scratch RAM**, and every single column reloaded both
pointers into `HL`, dereferenced, and wrote them back — plus it copied each
4-byte state slot into a scratch (`ST_H`) before adding it to the accumulator.

**Why the fix worked.** Two software pointers reloaded/rewritten per column is
~30 cycles/weight of pure bookkeeping on an 8-bit CPU with only one 16-bit
indirect register. Holding the weight cursor in `HL` (walked with `ld a,(hl+)`,
which auto-increments for free) and the state cursor in `DE` keeps both live
across the whole inner loop — the per-column pointer save/restore vanishes. Folding
the state read directly into the 4-byte accumulate also drops the `ST_H` scratch
round-trip (a whole redundant pass over 4 bytes per weight). Net ~157 → ~68
cycles/MAC on this matvec, same integer result.

**Why it's safe.** The weights for a bank are laid out contiguously in the mapped
`[0x4000,0x8000)` window (the caller switches banks between calls, never mid-
matvec), so a simple incrementing `HL` cursor walks them correctly. Same ordered
ternary add/sub, same skip-on-zero, same encoding (`1 => +1`, other `=> -1`).

---

## Entry 2 — state out-projection: HRAM accumulator + `add a,(hl)`

**File:** `gbf-kernel/src/asm_impl_state.rs` (`emit_state_out_matvec`, `emit_acc4_state_pm`)
**Effect:** 18.707 → 18.214 s/char (−2.6%). `smv_col` 7.3% → 6.4%.

**What it was.** After entry 1 the inner loop still (a) read each state byte into
a temp register before adding it to the accumulator, and (b) accessed the 4-byte
accumulator with absolute addressing (`ld a,(nnnn)` / `ld (nnnn),a` = 4 M-cycles
each).

**Why the fix worked.** Two independent addressing wins, both numerics-neutral:
- **Swap the cursors** so the *state* cursor is in `HL`. Then the state byte is
  consumed directly by the ALU as `add a,(hl)` / `adc a,(hl)` (and `sub`/`sbc`),
  eliminating the separate `ld a,(de); ld b,a` load+move per byte. The weight
  cursor moves to `DE` (`ld a,(de); inc de`), costing +2 cycles/weight on the
  fetch but saving more on the accumulate.
- **Move the accumulator into HRAM** (`0xFF80..=0xFF83`). High RAM has the
  shorter `LDH` encoding (3 M-cycles vs 4 for absolute), so all 8 accumulator
  touches per weight get cheaper. HRAM is otherwise unused by the kernel and the
  stack lives at `0xDFF0` (WRAM), so there's no collision.

Together: ~68 → ~53 cycles/weight on this matvec. Bonus simplification — the
row-end store to `(OPTR)` no longer needs to preserve a cursor (it clobbers the
`HL` state cursor, which is reset next row anyway; `DE` is untouched), so the
`push/pop hl` from entry 1 is gone.

**Why it's safe.** Same adds in the same order; only the operand addressing
changed. Guarded by the byte-exact regression + arm-B regression.

---

## Entry 3 — `mul16x8`: zero-multiplier early-out

**File:** `gbf-kernel/src/asm_impl_model.rs` (`emit_mul16x8`)
**Effect:** 18.214 → 17.782 s/char (−2.4%).

**What it was.** `mul16x8` (the shared 8×16→24 shift-add multiply, called ~4×/lane
by the norm's sum-of-squares plus decay/scale epilogues — the hottest driver
*page*) always ran all eight shift-add iterations, even when the multiplier byte
`A` was 0.

**Why the fix worked.** For a zero multiplier the product is zero and the eight
iterations are pure no-ops (the loop shifts a zero multiplier, never adding the
multiplicand). The high byte of the multiplier is frequently zero — small
quantized activations, Q8.8 scales whose integer part is 0, i24 values under
2^16 — so an `or a; ret z` at the top (result `C:HL = 0` is already loaded)
short-circuits those calls. The measured 2.4% drop confirms a meaningful
fraction of calls hit it; the ~3-cycle test added to the non-zero path is
dwarfed by the ~40 cycles saved per zero call.

**Why it's safe.** Returns exactly the value the loop would have produced (0) for
the only input it short-circuits. Byte-exact: d192 + arm-B regressions and all
63 kernel unit tests pass.

## Historical floor analysis for this cartridge

After entries 1–3 (21.695 → 17.782, −18.0%), every remaining block was traced to
its algorithmic limit for byte-exact integer work on the LR35902:

- **V3 matvec — 37.7%, the largest block, at its floor.** Each nonzero ternary
  weight adds an 8-bit activation into a 16-bit accumulator. That is 6 M-cycles
  and cannot be fewer: the LR35902 has no "add 16-bit += 8-bit" instruction, so
  the branchless `add/ld/adc/sub/ld` idiom (or any equivalent) is 6 ops. With
  real weights only 0.59% zero there is no sparsity to skip. 37.7% is just the
  irreducible cost of ~974k ternary MACs + the `pop`-per-pair activation fetch.
- **State out-projection — ~10%.** Optimized twice (entries 1–2); now register-
  resident cursors + HRAM accumulator. At its floor for an i24-activation matvec.
- **RMS norm — ~10%.** Dominated by the 24-bit sum-of-squares (a genuine square
  per lane, 3 sub-multiplies). The multiplier-zero early-out (entry 3) already
  collapses the hi-byte products for small lanes; the 7-byte i48 accumulator is
  the minimum width for 192 summed i24 squares; the `isqrt48` is a fixed 24-step.
- **Tied head — ~9%.** Already register-pointer-tight in this legacy profile:
  the per-lane product LUT is built with adds (not multiplies) and its
  256×d_model logit accumulate uses
  `add a,(hl)` against `(bc)`/`(de)` cursors. No addressing slack.
- **Per-row epilogues (up/down/state-out) — small.** They keep read/write cursors
  in scratch RAM, but the reload is only ~0.5% combined and is forced by the
  `mul16` call in the loop body clobbering the register file — reward below risk.

Considered and rejected: a **quarter-square multiply table** (`a·b = f(a+b) −
f(|a−b|)`). It is exact, but it does *not* win here — `mul16x8` already does the
full 8×16 product in one 8-iteration pass, whereas the table form needs two 8×8
products plus 9-bit-index address arithmetic, i.e. more work, especially now that
the zero early-out is in.

Further byte-exact speedups exist only as sub-1% micro-opts with rising risk. The
real next levers are strategic, not micro:

## Remaining levers recorded at the time

- **RMS norm cluster** (`n24_*` / `udn5_rot_*` / `udiv_norm5`, ~10%): runs ~8×/token.
  The 24-bit sum-of-squares (three software multiplies per lane) is intrinsic;
  gains would need a squaring-specific routine — higher risk.
- **Tied-head logits** (`emit_head` / `hg_*`, ~8%): this legacy profile builds
  a per-lane product LUT and accumulates 256 logits. The current V1024
  interactive cartridge uses paged logits and SRAM-full storage instead.
- **Software multiplies** (`mul16` / `mul16x8`): zero-multiplier early-out landed
  (entry 3). The remaining shift-add cost is the biggest structural prize — a
  quarter-square table (`a*b = f(a+b) - f(|a-b|)`, `f(n)=floor(n^2/4)`, exact for
  all bytes; ~1 KB, fits in the always-mapped bank 0) would roughly halve every
  multiply and help the norm, decay, scales, and head at once. Larger, bank-
  placement-sensitive change — deserves a dedicated pass, not a micro-step.
- **V3 matvec** (~37%, largest single block): already near its floor; the zero-skip
  machinery is near-useless on the real 0.59%-zero weights but removing it saves
  little. Bigger structural change would be the V2-dispatch path (trades speed for
  capacity — out of scope for pure speed).
