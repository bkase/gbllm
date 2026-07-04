# F-S8 matched-cycles sizing sweep (bd-3771m)

Generated from `report.json` (schema `s8_matched_cycles_sweep.v1`, git 0efc87b3ac54, seed 0).
Bin: `gbf-experiments/src/bin/s8_matched_cycles.rs` (reuses the s5_state_ab substrate:
charset_v1 80-id vocab, LinearState MT4 + residual, pre-norm ternary FFN stack,
tied head, warmup Off->Hard QAT, TBPTT-128 x 4 lanes, seed 0).

## UX budget (hard facts, measured constants)

- 30 s/char at 70% CPU = 22.0M M-cycles/token.
- V3 weights-as-code: 5.385 cy/MAC, 4.401 B/weight -> 4.09M MACs/token envelope.
- V2 dispatch: 10.261 cy/MAC, 0.25 B/weight data + 2699 B shared code -> 2.15M MACs/token envelope.
- ROM budget: 7 MiB (8 MiB MBC5 minus ~1 MiB runtime/UI reserve).
- Constants source: docs/experiments/kernel-bakeoff/kernel_bakeoff.json @ 40% zeros (V3 5.385 cy/MAC, 4.401 B/w; V2 10.261 cy/MAC, 0.25 B/w + 2699 B shared code).

## Quality bar (KN-5, copied verbatim from experiments/S4/baseline/)

- KN-5 full corpus (466 MiB train): **2.2584 bits/raw-byte** (2.2718 bpc/char).
- KN-5 64 MiB train cap (the cap the Phase-A arms trained on): 2.3264 bits/raw-byte (2.3402 bpc/char).
- Val stream: books [1017, 1233, 1475, 2105, 2156, 3440], raw sha `e31abc36189a319d...` (identical across all arms and KN-5).

## Per-arm results (full val stream, 1,042,400 scored pairs)

| arm | topology | bits/raw-byte (hard ternary) | bits/raw-byte (fp soft) | QAT gap (bpc/char) | MACs/token | V3 ROM (MiB) / s-char | V2 ROM (MiB) / s-char |
|---|---|---|---|---|---|---|---|
| A1 | d128/ff256/4blk | **2.9912** | 2.9783 | 0.0130 | 305,152 | 1.28 / 2.2 | 0.12 / 4.3 |
| A2 | d160/ff320/6blk | **3.0342** | 3.0152 | 0.0191 | 678,400 | 2.85 / 5.0 | 0.22 / 9.5 |
| A3 | d128/ff256/4blk/E4 top-1 | **3.1623** | 3.1638 | -0.0015 | 307,200 | 4.59 / 2.3 | 0.31 / 4.3 |
| A1_distill | d128/ff256/4blk + distill | **2.8883** | 2.8263 | 0.0624 | 305,152 | 1.28 / 2.2 | 0.12 / 4.3 |

Method: bits/raw-byte = bpc_per_normalized_char x (val_chars_total / val_raw_bytes_total),
the same total-bits-over-raw-byte-count re-expression as the KN-5 artifact.
Samples: `sample_<arm>.txt` (256-char greedy continuations, hard ternary).

## Distillation probe

- Teacher: fp dense teacher for A1: 2x width, QAT hardness Off + act passthrough for all steps -> fp val 2.6694 bits/raw-byte after 20000 steps (32 min).
- Loss: CE + weight * T^2 * softCE(softmax(teacher/T) || softmax(student/T)), teacher logits recomputed per step on the training batch with per-lane teacher state carry (T=2.0, w=0.5).
- Student-with-distill 2.8883 vs student-without 2.9912 at matched 40000 steps: **delta -0.1029 bits/raw-byte** (distill helps).

## Verdict

- Best deployable arm: **A1_distill** at 2.8883 bits/raw-byte (hard ternary).
- Beats KN-5 full corpus (2.2584)? **False** at this proxy scale.

## Overnight scale run

- Config: d192/ff384/6blk/slots192/E1 (CUSTOM arm of the same bin).
- Projected: 973,824 MACs/token; 958,464 stored ternary weights;
  V3 4.09 MiB / 7.1 s/char (fits 7 MiB: True);
  V2 0.30 MiB / 13.6 s/char (fits 7 MiB: True).
- Recipe: the Phase-B winner (distillation): fp dense teacher d384/ff768/6blk/slots384
  trained 30k steps, then the student distilled for 130k steps (T=2.0, w=0.5),
  train cap 256 MiB, seed 0. Launched via:
  `nohup ./target/release/s8_matched_cycles --phase distill --arm CUSTOM --d-model 192
  --d-ff 384 --n-blocks 6 --state-slots 192 --n-experts 1 --steps 130000
  --teacher-steps 30000 --teacher-mult 2 --train-cap-bytes 268435456
  --out-dir experiments/S8/sweep > experiments/S8/sweep/scale_run.log 2>&1 &`
- Log: `scale_run.log`; result lands in `arm_CUSTOM_distill.json` +
  `sample_CUSTOM_distill.txt` when done (~10-11 h).
- Monitor: `tail -f experiments/S8/sweep/scale_run.log` (teacher finishes ~2 h in;
  look for `[distill] teacher fp val:` then `[CUSTOM] step N/130000` lines).

## Caveats

- Proxy scale: 40k steps x 512 tokens/step (20.5M tokens) on the 64 MiB train prefix;
  matched steps relatively undertrains the larger arms (A2, A3), so Phase A measures
  the architecture signal at this budget, not the asymptote. The scale run probes further.
- The MoE router is a simplified in-bin fp linear top-1 (documented in the bin header),
  not the gbf-model Top1RouterQat low-rank core; hard top-1 dispatch is stop-gradient
  provenance, router gradients flow via top-1 prob output scaling + Switch aux (w=0.01).
  A3 showed partial router collapse in block 3 (one dead expert) despite balanced blocks 0-2.
- ROM accounting: ternary weights at the measured B/weight constants; embedding f32 (4 B),
  scales/decay Q8.8 (2 B), router int8 (1 B, assumption). Norms, dispatch overhead beyond the
  shared V2 handlers, and WRAM/SRAM state cost are excluded (same caveats as the bakeoff).
- Eval scores within-lane adjacent pairs (8 lanes); KN-5 scored every token under reset-context
  windows. The bits/raw-byte re-expression uses the stream-level chars-per-raw-byte ratio.
