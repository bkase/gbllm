# Review Order

1. Read `gbf-hw/src/cartridge_header.rs` and the small `gbf-asm/src/rom.rs` re-export diff.
2. Read `gbf-hw/src/target.rs` to understand the bring-up profile and cartridge validation.
3. Read `gbf-hw/src/memory.rs`; this is the most load-bearing contract.
4. Skim `gbf-hw/src/mbc5.rs`, `lcd.rs`, `timing.rs`, `interrupts.rs`, and `joypad.rs`.
5. Read `gbf-hw/src/calibration.rs` for constructor and serde validation.
6. Check `gbf-hw/tests/cross_module_conformance.rs` and `single_source_smoke.rs`.
7. Use `claim-to-gate.md` to map each claim to its test.
