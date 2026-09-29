# gbllm kinetic film: production note

**Shareable deliverable:** `gbllm-kinetic-share.mp4`. 74 s, 1920×1080, 60 fps, H.264 High/yuv420p (BT.709),
AAC 48 kHz stereo at 256 kb/s, faststart, about 50 MB. The higher-bitrate local master is
`gbllm-kinetic.mp4` (about 125 MB, ignored by Git). The audio is mixed to about −14.6 LUFS
integrated with a −1.0 dBTP true peak.

**Editable source:** `src/`. The film is code: a single Canvas-2D scene graph that is a pure function of
time. Scrub it live in Chrome by opening `src/film.html` (space bar plays it with the score; append
`#40` to the URL to start at 40 s). `src/build.sh` rebuilds both videos from the checked-in
`data.js`. Set `GBLLM_TOKENIZER` to the pinned tokenizer to regenerate `data.js` from the
committed acceptance evidence.

| File | Role |
|---|---|
| `src/timeline.js` | The shared cue sheet (120 BPM) that drives both picture and sound. Retiming happens here. |
| `src/film.js` | All scenes, copy, layout and motion. |
| `src/make_data.py` | Builds `data.js` from the committed acceptance evidence and checks the pinned tokenizer hash. |
| `src/score.mjs` | An original synthesized score and sound design. It uses no samples or third-party audio. |
| `src/render.mjs` | Headless Chrome over DevTools (no npm deps) → PNG frames → ffmpeg. `--stills 12,40` gives quick checks. |

## Creative direction

The film takes its look from the two mood references: a dark stage, warm amber letter "motes" and
editorial condensed type. It uses cream for the plain voice, lime for the payoff and amber for the
machine and the numbers. It sets two kinds of image apart on purpose:

- **Conceptual layer:** big Helvetica Neue Condensed Black, PragmataPro technical labels, particles
  and diagrams. None of this is presented as device output.
- **Authentic layer:** the real 160×144 framebuffer, shown once as the reveal. It is scaled 6× with
  nearest-neighbor only. The scene has no grain, vignette, shake, flash, tint or overlay on the
  frame. It carries the label "framebuffer from the accepted ROM run · unretouched".

The film follows the hook, then the mechanism, then the proof. It runs at 120 BPM with word slams on
the beat.

| Time | Section | Beat |
|---|---|---|
| 0–4 | Cold open | A cursor types the real prompt, `Once upon a time`. |
| 4–12 | Hook | "A LANGUAGE MODEL / RUNS ON AN ORIGINAL / GAME BOY." and "(yes, the one from 1989.)". Then: no network, no host PC, no coprocessor, "just the cartridge". |
| 12–20 | The machine | 8-BIT, 8 KiB, NO FPU (3.14159 loses its decimals) and 8 MiB each fill the screen, then dock into a spec strip. The section ends on "We didn't shrink a model to fit. We built one for the machine." |
| 20–46 | The build | Seven stations on a progress rail: bytes → BPE (real IDs 435/443/258/402) → teacher d512 distilled into student d192 → hard QAT snap at step 9,600 → ternary projection weights become add/sub/nothing → 408 of 512 banks → a small runtime that alternates think and draw at token boundaries. |
| 46–54 | The loop | The on-cartridge loop slams one word per beat. Then the music tape-stops for the "22 SECONDS" gag: a progress bar fills in real time, "Give it a second… or twenty-two.", then a time-lapse cue. |
| 54–66 | Proof | The real framebuffer, next to a re-typeset time-lapse of the 24 real generated tokens, with a DMG generation clock that ends at 8:25. Closes on "SMALL. IMPERFECT. REAL." |
| 66–74 | Coda | "TRAINED. DISTILLED. QUANTIZED. COMPILED. CARTRIDGED." → "Once upon a time, there was a little Game Boy." → the `gbllm` wordmark. |

In the sound design, keystrokes are pulse blips. Word slams use a sub kick, LFSR noise and a chord stab.
At the QAT snap, a smooth rising tone is quantized live to {−1, 0, +1}. In the proof section, **each
generated token plays one note whose pitch comes from its real token ID**.

## Fact sourcing

Every figure on screen comes from `README.md`, `docs/experiments/interactive-subword/` or the brief.
`make_data.py` pulls the real token IDs and strings, the framebuffer pixels and the per-token M-cycle
timings from `acceptance-result.json`. It decodes the IDs with the production tokenizer and asserts that
tokenizer's SHA-256 (`ffae9160…`). The clock figures are derived:

- The generation clock: Σ `generationBoundaryMCycles` ÷ 1,048,576 M-cycles/s = 8:25.
- The time-lapse factor: 505 s ÷ 6 s ≈ 84×.

Two statements come from general hardware knowledge rather than the repo:

- The DMG CPU clock is 4.19 MHz. This is consistent with the repo's cycle-to-seconds numbers.
- The original Game Boy launched in 1989.

These claims were avoided on purpose:

- A full OS, or preemptive or mid-token multitasking. The film says "A tiny runtime… alternates
  inference with drawing, at token boundaries", "The screen holds still while it thinks" and
  "Not a full OS".
- Chat-assistant abilities. The film says "SMALL. IMPERFECT. REAL."
- Real-time generation. The film shows "22 SECONDS" per token and labels the fast section as a
  time-lapse.

## Compromises and caveats

- **No emulator capture.** SameBoy is not installed, and the local ROM's hash differs from the
  production hash. The only device imagery is therefore the committed verified framebuffer
  (sha256 `40fc29f2…` of `framebuffer.pgm`). The proof scene's right-hand token stream is a
  re-typeset reconstruction from the logged token IDs. It is not footage, and it is labeled as a
  time-lapse.
- **The assembly listing is illustrative.** The instruction sequences follow the shape of the add and
  subtract accumulate emitters in `gbf-kernel/src/asm_impl_state.rs`. However, the production ROM uses the
  V3 straight-line lowering, and the film labels the panel "illustrative excerpt" instead of claiming
  to show a literal ROM dump.
- **H.264 4:2:0 encoding.** The framebuffer is pure black and white, and at 6× every source pixel is a
  6×6 block, so it survives encoding visually intact. The luma values are not bit-exact after lossy
  compression. For bit-exact pixels, use `refs/03-verified-rom-frame.png` or `framebuffer.pgm`.
- **Fonts.** Helvetica Neue Condensed Black ships with macOS. PragmataPro is the author's locally
  licensed font; on another machine the source falls back to Menlo, so the render will differ slightly.
- The AI-generated incidental text in `refs/02` was not reproduced anywhere.
