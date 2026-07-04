# Kernel bake-off: ternary matvec strategies on DMG (bd-rzq5n)

**Date:** 2026-07-04. **Producer:** `cargo run -p gbf-bench --bin kernel-bakeoff`.
**Evidence:** `kernel_bakeoff.json` / `kernel_bakeoff.md` in this directory
(regenerated deterministically; fixed seeds, gameroy-measured M-cycles,
outputs byte-verified against `gbf_kernel::ref_impl` before any timing is
reported).

## Question

The project's load-bearing feasibility unknown (2026-07-02 review, critiques
#1/#2): what does one ternary MAC actually cost on the LR35902, per kernel
strategy, and which registered model profile is interactive at that cost?

## Strategies measured

| id | strategy | weights live as | measured cy/MAC @ 40% zeros |
|---|---|---|---|
| V1 | interpreted `Ternary2` decode (generic loop) | data (0.25 B/w) | **28.4** |
| V2 | threaded per-byte pattern dispatch, 81 handlers | data (0.25 B/w + 2.4 KiB shared code) | **10.3** |
| V3 | weights-as-code (straight-line add/sub, zero skipped) | code (~4.4 B/w @ 40% zeros) | **5.4** |

Sparsity sweep (0 / 400 / 600 / 900 permille zeros): V3 scales from 7.8 down
to 1.4 cy/MAC; V1 barely moves (30.8 → 25.4) because decode dominates; V2 sits
between (13.3 → 6.5).

## Projected tokens/sec (matvec floor, 70% CPU after UI reserve)

| profile | MACs/token | V1 | V2 | V3 | V3 s/char |
|---|---|---|---|---|---|
| Toy1 (d32/ff64/2blk) | 12,800 | 2.0 | 5.6 | **10.6** | 0.09 |
| MoeTiny (d64/ff128/4blk) | 87,040 | 0.30 | 0.82 | **1.57** | 0.64 |
| UpperBank-96 (d96/ff192/4blk) | 192,000 | 0.13 | 0.37 | **0.71** | 1.4 |
| UpperBank-128 (d128/ff192/4blk) | 272,384 | 0.09 | 0.26 | **0.50** | 2.0 |
| QualityDense (d144/ff288/6blk) | 633,600 | 0.04 | 0.11 | **0.22** | 4.7 |
| QualityDense (d160/ff320/6blk) | 780,800 | 0.03 | 0.09 | **0.17** | 5.7 |

These are floors: norms, router, per-row scale application, decode, yield
overhead, and bank switching are all excluded (see report caveats).

## UX budget (updated 2026-07-04)

bkase's directive: **quality over speed — up to ~5 s/char is acceptable.**
That sets the per-token budget at ~3.67M M-cycles (5 s x 1,048,576 x 70%),
i.e. **~680k matvec MACs/token under V3**. Sizing consequences:

- The whole registered profile ladder (Toy1 → UpperBank-128) is comfortably
  inside budget under V3; the binding constraint moves from cycles to
  **capacity per ROM byte**.
- The dense frontier at this budget is roughly d144/ff288/6-block (~634k
  MACs/token, ~4.7 s/char floor); d160/ff320/6 is just over.
- **MoE re-enters** under the new constraint: top-1 routing stores k experts
  but spends one expert of cycles per token, so at fixed cycles/token MoE
  buys capacity with ROM (V3 ≈ 4.4 B/weight, 8 MiB MBC5 ceiling ≈ ~1.8M
  weights as code). The S7 dense-vs-MoE verdict was pinned at matched
  deployed *bytes*; the decision-relevant comparison is now matched
  *cycles/token* with a ROM ceiling. Revisit before S8 sizing.

## Findings

1. **Weights-as-code (V3) dominates everywhere measured** — 5.3x faster than
   the interpreted kernel the size-profile registry implicitly assumed, and
   the only strategy that keeps even MoeTiny near-interactive. Under the
   quality-first budget it is what makes UpperBank-and-larger profiles viable
   at all.
2. **The V3 ROM trade is affordable.** ~4.4 bytes/weight at 40% zeros puts
   all of MoeTiny's ~284k stored weights at ~1.2 MiB of generated code —
   inside the 2 MiB objective cap and the MBC5 8 MiB ceiling. Zero-skipping
   means sparsity buys both speed and ROM back (0.9 zeros → 1.4 cy/MAC,
   ~1.1 B/w).
3. **Dispatch (V2) is the capacity extender.** 0.25 B/w like V1 but 2.8x
   faster; its cost is dominated by the 14-cycle threaded dispatch (the
   LR35902 has no indexed jump). Under the quality-first budget a mixed
   lowering — V3 for hot mix/head layers, V2 for cold expert bulk — is the
   natural way past the 8 MiB weights-as-code ceiling.
4. **Architecture implication (planv0):** the compiler needs a
   weights-as-code lowering path for hot matvecs. That blurs the
   `TargetDataLoweringArtifact` (data) vs compile-product (code) boundary —
   generated weight-code is both — and largely dissolves `RomWindowPlan`'s
   kernel-vs-tensor co-residency problem for those layers, since the expert
   payload *is* the kernel.

## Measurement notes

- Fixture: 64 fan-in x 32 rows (2,048 MACs), one ROM bank, interrupts off,
  activations pre-staged in WRAM; SP is repurposed as the activation stream
  pointer in V2/V3 (a bake-off simplification called out in the caveats).
- Cycle counts are gameroy M-cycle deltas between the kernel start/end PCs,
  after each ROM's 32 x i16 outputs byte-matched the exact host reference.
- Numeric core: unsigned accumulate at zero point 128 with per-row constant
  `-128 * sum(row)` folded into the accumulator seed; branchless 16-bit
  add/sub idioms (6–9 cycles per nonzero MAC).

## Follow-ups this measurement motivates

- Streaming variant: same kernels with operands crossing a switched bank, to
  price the residency plans' bank-switch assumptions (owner: F-B1x/F-H2 line).
- Scale application epilogue (Pow2 shift vs Q8.8 software multiply) on top of
  V3, to close the numeric-contract question (Action 8 of the review).
- Promote V3 into a named kernel family under F-H2 (bd-3se9) and a
  `DataLoweringProfile`/lowering decision under the artifact contract.
