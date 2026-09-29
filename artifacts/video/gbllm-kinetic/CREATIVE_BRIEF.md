# gbllm kinetic typography film — creative brief for Claude Code Opus 5.5

## Commission

Make and render a finished, engaging kinetic typography hype video for this art project. Own the creative direction: motion language, shot structure, visual style, pacing, rhythm, sound design, and exact on-screen copy are yours. The references below are mood and material cues, not storyboards to reproduce. Deliver a playable MP4 in this folder plus editable source and a short production note. Do the work, not just a plan.

The central hook is the absurdity that a **real, learned, coherent-enough language model runs entirely inside an original Game Boy game cartridge**. The contrast should feel delightful and slightly unbelievable, then the detail should make it credible. Show what it took: training from scratch for the device, distillation and quantization, compiling weights into LR35902 instructions and banked ROM, and a bespoke cartridge runtime coordinating inference with LCD drawing.

## Reference images and proof

- `refs/01-language-constellation.png`: physical handheld against impossible scale; warm letter particles, dark stage. Mood only, not a real device screenshot.
- `refs/02-compiled-cartridge.png`: exploded editorial composition from language/data to cartridge to display. Mood only. It contains AI-generated incidental text; **do not reproduce or quote that text as fact**.
- `refs/03-verified-rom-frame.png`: real framebuffer from the current accepted ROM run, sourced from `docs/experiments/interactive-subword/framebuffer.pgm`. This is authentic output, not generated art. Original size 160x144. Use exact pixels if presenting it as actual game output.
- `docs/experiments/interactive-subword/transcript.txt`: the verified output for prompt `Once upon a time` begins `, there was a little girl named Lucy. She loved to play with her ball...` (the terminal line breaks are display wrapping).
- `README.md` and `docs/experiments/interactive-subword/README.md` are the factual source of truth. Read them before scripting.

## Factual anchors

- The production cartridge is 8 MiB MBC5+RAM, with 8 KiB cartridge SRAM. Original Game Boy has an 8-bit LR35902, 8 KiB internal work RAM, and no floating-point unit.
- The deployed dense student uses width 192, FFN width 384, six blocks, 192 recurrent state slots, a 1,024-entry byte-BPE vocabulary, and about 1.16 million trained parameters. It is a small story-like language model, not a modern chat assistant.
- A larger full-precision MLX teacher was trained from scratch, then a smaller student was trained/distilled for this hardware. Student training has 24,000 steps with hard quantization-aware training after step 9,600; learned projections harden to ternary -1/0/+1 with row scales. Activations are trained for an Int8 grid.
- Build path: TinyStories bytes -> byte-BPE -> MLX teacher -> distilled/QAT student -> hardened checkpoint -> Python artifact bridge -> `gbf compile --profile interactive-subword-dmg` -> integer lowering -> specialized LR35902 instructions -> `gbf-asm` -> Game Boy ROM. The compiler specializes ternary weight rows into additions, subtractions, or no operation, rather than general multiplications in the hot path.
- Joypad inputs a prompt. The cartridge itself tokenizes, maintains recurrent context, runs six blocks and 1,024-way head, samples top-k 4 at T=0.6, feeds the selected token back, and draws text. No network or host inference coprocessor. The verified prompt `Once upon a time` yields 24 tokens. Measured steady generation is roughly 22 seconds per token at normal DMG speed. If showing generation rapidly, mark it as accelerated/turbo or a time lapse.
- Runtime caveat: the shipped v0 shell is bespoke OS-like code coordinating compute, cartridge bank switching, and LCD drawing. It renders at token boundaries, synchronizes VRAM writes with VBlank, and leaves interrupts disabled during the session. The screen is static during a forward pass. **Do not claim a full preemptive OS or mid-token cooperative task switching exists.** A truthful line is "We built a tiny runtime to alternate inference with drawing at token boundaries." The full cooperative scheduler remains future work.
- The model produces recognizable, context-sensitive English-like story continuations. Avoid claiming ChatGPT-scale knowledge, perfect coherence, or instruction following.

## Production guidance

- Keep the conceptual imagery and authentic game pixels visually distinct. The actual framebuffer is especially potent as a reveal/proof moment.
- A SameBoy capture is optional. SameBoy was not found in PATH or Applications during production. A local ROM candidate had a SHA-256 that differed from the production hash `230a3842...` documented in README; do **not** present a capture from it as the verified production artifact without resolving the identity. Prefer the committed verified framebuffer if that cannot be resolved quickly. Do not wait 22 seconds per token for footage; use turbo/fast emulation if acquiring genuine video.
- Original audio/synthesized SFX are welcome. Avoid third-party music or footage without clear rights.
- Favor strong readable typography and precise technical copy. Make the title/hook quick, the mechanism vivid, and the ending memorable. Duration and aspect ratio are your editorial decisions, but export a standard 16:9 H.264 MP4 suited for sharing.
- Render the final MP4, verify playback metadata with `ffprobe`, and report the paths and any compromises.
- Keep new work contained to `artifacts/video/gbllm-kinetic/`. Do not change project source or beads. The local `br` database currently fails with schema mismatch; don't attempt migration for this video task.
