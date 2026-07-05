# Fidelity fix: wide down-delta carrier (bd-2vkqt, `state-int-semantics.v2`)

The v1 run of this evidence (git `530a2218`, preserved as
`report.v1-before-down-delta-fix.json` / `README.v1-before-down-delta-fix.md`
plus the `*.v1-before-down-delta-fix.txt` samples/transcript) passed every
byte-exact gate but scored the canonical integer path at **4.6803 bpc/char vs
the f32 trainer port's 2.7786** (+1.9017 bpc, argmax agreement 63.77%). The
standout counter was `down_delta_clamp_events = 1,040,285` (~1 per position):
the down-projection residual delta was carried in a u16 clamped at 65,535 raw
(2,047.97 units) — a Q11.5-era carrier width that was never re-proven when the
residual widened to i24 Q19.5.

Everything in this document is program output; nothing is hand-computed.

## 1. Measured first (`down_delta_probe.v1`, bin `d192-down-delta`)

The host evaluator was instrumented with a semantics-neutral
`DownDeltaProbe` (32-raw-unit histogram of the **unclamped** delta magnitude,
recorded before the v1 cap) and run over the **full committed pair set**
(1,042,400 positions, 8 lanes, 1,200,844,800 deltas) on the real
`checkpoint-export-CUSTOM_distill` weights, pre-fix semantics:

| quantity | raw Q19.5 | units (raw/32) |
|---|---|---|
| max unclamped delta | 308,033 | 9,626.03 |
| p99 | 639 | 20.0 |
| p99.9 | 8,031 | 251.0 |
| p99.99 | 280,415 | 8,763.0 |
| v1 cap | 65,535 | 2,047.97 |
| deltas >= cap | 1,040,285 of 1.2e9 (0.0866%, ~0.998/position) | — |
| structural per-row bound (from actual weights/scales) | 3,023,076 | 94,471 |
| signed-i24 carrier bound | 8,388,607 | 262,144 |

Cross-checks: the probe's count above the cap equals the filed clamp counter
exactly; the arm-B checkpoint's structural bound (358,026) also exceeds the
u16 cap, which is why the carrier-width decision is keyed to the
accumulator-width structural bound (keeping arm-B's i16 path bit-identical),
not to a "delta bound > 65,535" routing rule.

## 2. The fix

`state-int-semantics.v2` (constant `STATE_INT_SEMANTICS_VERSION`,
`gbf-kernel/src/state_model_ref.rs` module docs carry the changelog):

- **Wide (i24-accumulator) path**: the Q19.5 delta
  `sign(m) * ((2|m| + 127) div 254)` is carried **exactly in a signed i24
  with no clamp**. Lowering proves the structural per-row bound
  `max_row floor((2*scale_raw*acc_bound + 127)/254)` from the actual ternary
  weights fits `2^23 - 1` and refuses to lower otherwise
  (`StateModelError::DownDeltaEscapesI24`). For this checkpoint:
  3,023,076 <= 8,388,607 (2.77x structural margin; the observed max has 27x).
- **i16-accumulator path (arm-B)**: untouched — same u16 carrier, same
  canonical counted 65,535 clamp. The arm-B one-token and multi-token ROMs
  built before and after the fix are **sha256-identical**
  (`755695fd…`, `e78bc9c2…`), and the full arm-B evidence runner re-passed
  every gate (one-token byte-exact, 4 multi-token seeds, f32 port reproduces
  the committed bpc).
- **ROM**: `down_ep24w` now calls the new `udiv254w` (24-iteration long
  division, exact u24 quotient) and applies the 3-byte delta directly; the
  i16 emission still selects the original `udiv254` path. Host evaluator and
  ROM implement the same function, re-proven byte-exact by the one-token
  (5/5 poked-state cases), multi-token (4 seeds x 128 tokens), shell-session
  and sample gates below, plus a new emulated regression test that boosts
  synthetic down scales 64x to force deltas past the old cap
  (`wide_down_delta_rom_matches_host_above_the_old_u16_cap`; this test
  caught a register-clobber bug in the first `udiv254w` draft before it
  shipped).

The synthetic d192-readiness model's structural delta bound is 23,308 — it
never touched the v1 cap, which is why only the real checkpoint exposed this.
All synthetic readiness gates re-pass under v2.

## 3. Before / after (full committed pair set, identical gates)

| metric | v1 (before) | v2 (after) | trainer f32 port |
|---|---|---|---|
| int val bpc/char | 4.6803 | **2.9883** | 2.7786 |
| int minus f32 (bpc) | +1.9017 | **+0.2097** | — |
| int bits/raw-byte | 4.6528 | **2.9707** | 2.7622 |
| argmax agreement vs f32 | 63.77% | **73.32%** | — |
| down-delta clamp events | 1,040,285 | **0** | n/a (unclamped) |
| max abs down delta applied (raw) | 65,535 (capped) | 308,033 (exact) | — |
| max abs residual (raw, i24 bound 8,388,607) | 72,347 | 310,443 | — |
| residual i24 wraps / state clamps | 0 / 0 | 0 / 0 | — |
| one-token mean M-cycles | 21,884,284 | 22,712,942 | — |
| generation-loop mean M-cycles | 21,886,637 | 22,715,505 (+3.79%) | — |
| seconds/token (DMG) | 20.873 | 21.663 | — |

The deployed integer d192 model (2.9883 bpc) is now decisively better than
the deployed arm-B d64 (3.30 bpc), as the trainer numbers said it should be.

## 4. Sample quality (same three settings, fresh shell-verified runs)

v1 samples degenerated (T0.6 collapsed into a `zzzz…` run; T0.8 into
all-caps consonant salad). v2 samples at the same settings/seeds/prompts are
coherent pseudo-English throughout; each is prefix-verified on-device by a
full scripted shell session (180-181 chars) before the host continues the
stream. See `sample_*.txt` (after) vs `sample_*.v1-before-down-delta-fix.txt`
(before).

## 5. Provenance

- After: `report.json` / `README.md` (schema `d192_real_bringup.v2`,
  `int_semantics_version = state-int-semantics.v2`), generated by
  `cargo run --release -p gbf-bench --bin d192-real`.
- Before: `*.v1-before-down-delta-fix.*` copies of the v1 outputs
  (schema `d192_real_bringup.v1`, git `530a2218`).
- Delta measurement: `cargo run --release -p gbf-bench --bin
  d192-down-delta` (`down_delta_probe.v1`); full-set outputs committed as
  `down-delta-probe.v1-semantics.json` (pre-fix semantics, the section-1
  numbers) and `down-delta-probe.v2-semantics.json` (post-fix semantics:
  identical tail, `down_delta_clamp_events = 0`).
- The committed `docs/experiments/d192-readiness` and
  `docs/experiments/stateful-rom` evidence predates v2 (and, for cycles,
  predates the d192 ROM parameterization); both runners were re-executed to
  verify all their gates still pass, but their regenerated outputs are owned
  by other beads and were not committed here.
