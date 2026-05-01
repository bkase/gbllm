# Review Order

Pass 0: run `./scripts/review/f-a1/verify-packet.sh`, then read `README.md`, `reproducibility.md`, and `generated-artifacts.md`.

Pass 1: type-state and API boundaries: `section.rs`, `lowering.rs`, `lib.rs`.

Pass 2: byte correctness: `encoder.rs`, `cycle_model.rs`.

Pass 3: placement and relaxation: `layout.rs`, `relax.rs`.

Pass 4: ROM builder: `rom.rs`, `gbf-asm/examples/tiny_rom.rs`.

Pass 5: listing and symbols: `listing.rs`, `symbols.rs`.

Pass 6: packet artifacts: `artifacts/tiny_rom.lst`, `artifacts/tiny_rom.sym`, and the SHA file. Do not line-review `tiny_rom.gb`; review the builder and reproducibility gate.
