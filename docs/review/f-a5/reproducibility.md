# Reproducibility

Regenerate the review artifacts with:

```bash
python3 tools/font/build_font.py
cargo run -p gbf-runtime --example demo_bank0_rom -- target/review/f-a5
cargo run -p gbf-runtime --example render_demo_screen -- docs/review/f-a5/demo-screen.png
```

The normalized Bank0 image is built from the same section builders as the runtime API, with fixed F-A5 placement anchors, branch relaxation, the Nintendo logo/header region stamped, global checksum bytes zeroed, and the F-A3 `BuildIdentityBlock` lineage hash fields zeroed.

The PNG is a 4x nearest-neighbor rendering of the real 160x144 `gbf-emu` framebuffer after booting the F-A5 demo ROM and placing the `FA5 OK` visual-review prompt into the bootstrapped BG map. The F-A5 M0 font is the deterministic petabyt/font-derived 7x5 bring-up font from `tools/font/build_font.py`, so the prompt verifies the expected glyph cells with readable ASCII letterforms.
