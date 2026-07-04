# S5 state A/B: LinearState multi-timescale vs stateless, on charset-80

Beads: bd-29ai4 (state A/B) + bd-2nrnq (charset-80 trainer migration).
Producer: `gbf-experiments/src/bin/s5_state_ab.rs` (features `s7`), seed 0,
40,000 matched steps per arm, 512 tokens/step per arm, same ternary QAT recipe
as `s2_gap_and_export.rs` (25% warmup Off -> Hard). Full artifact:
`report.json` (schema `s5_state_ab.v1`); raw log: `run.log`.

All numbers are deterministic evals over the full shared val stream
(books [1017, 1233, 1475, 2105, 2156, 3440], raw sha `e31abc36...`,
1,042,400 scored pairs, hard ternary unless noted).

| model | bpc / normalized char | bits / raw byte | fp-vs-ternary gap (bpc) |
|---|---|---|---|
| A stateless bigram-context FFN (charset-80) | 3.6651 | 3.6435 | -0.0006 |
| B LinearState MT4, TBPTT seq 128 x 4 lanes | 3.1994 | 3.1806 | +0.0471 |
| C = B with seq 256 x 2 lanes | 3.1774 | 3.1587 | +0.0088 |
| KN-5 baseline, 64 MiB cap (reference) | 2.3402 | 2.3264 | n/a |
| KN-5 baseline, 256 MiB cap (reference) | 2.2851 | 2.2717 | n/a |
| old byte-256 stateless toy (gap.json, reference) | n/a | 3.6687 | n/a |

## Verdict

**Real sequence state wins decisively at matched d64 capacity.** Arm B beats
the stateless baseline by **0.466 bpc/char** (3.199 vs 3.665; per raw byte
3.181 vs 3.644) with identical d_model/d_ff/n_blocks/steps/seed. The longer
BPTT context (arm C) adds a further 0.022 bpc/char. The recurrent state — 64
f32 slots with fixed decay bands [0.5, 0.75, 0.875, 0.9375] — is doing exactly
what planv0 predicted: carrying context past the bigram ceiling. Arm B's
canonical ternary tensors (including state-mix projections and exact Q8.8
decay slots) are exported under `checkpoint-export/` with the recurrence
semantics documented for integer reproduction.

**Nobody beats KN-5 yet.** Best neural arm (C, 3.159 bits/raw-byte) is still
0.83 bits/raw-byte above the KN-5 64 MiB baseline (2.326). Closing that gap
needs capacity/steps, not just state — this experiment deliberately stayed
d64-class to isolate the state question.

The charset-80 migration itself is roughly bpc-neutral at this scale: arm A
(3.6435 bits/raw-byte) reproduces the old byte-256 toy (3.6687) on the same
val stream, so vocabulary alone was not the bottleneck — state was.

## Sample quality (256-char greedy, hard ternary, fixed prompt)

- A: `sample_arm_A.txt` — immediate `" the the the ..."` loop (bigram argmax
  fixed point).
- B: `sample_arm_B.txt` — `" the come the care the care ..."` — forms a
  multi-word cycle; real words, still degenerate under argmax.
- C: `sample_arm_C.txt` — `" the the the ..."` loop.

Greedy argmax loops are expected at this bpc; the samples are committed as
readability evidence, not a quality claim.

## Honest caveats

- The stateful recurrence is a batched Burn re-implementation of the
  `LinearStateBlock` semantics (the committed `LinearStateBurnQat` adapter is
  single-sequence); parity against the canonical scalar QAT kernels was
  measured on-checkpoint: max abs diff 1.9e-6 (B), 2.4e-6 (C).
- Eval scores within-lane adjacent pairs (8 lanes; lane-first tokens
  unscored); bits/raw-byte re-expresses bpc via the stream-level
  chars-per-raw-byte ratio — same total-bits-over-raw-bytes method as the
  KN-5 artifact, which scored every token under reset-context windows.
- Arm A trains on iid pairs (as s2) while B/C train on sequential TBPTT
  lanes; steps and tokens/step are matched, data-order exposure differs.
- Arm B/C fp reference is the SOFT same-scale ternary relaxation (calibrated
  STE ceiling), matching the gap.json methodology.
- KN-5 numbers are copied verbatim from
  `experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json`, not re-run.
