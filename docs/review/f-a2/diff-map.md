# Diff Map

Deep review:

- `gbf-hw/src/cartridge_header.rs`
- `gbf-hw/src/target.rs`
- `gbf-hw/src/memory.rs`
- `gbf-hw/src/calibration.rs`

Boundary review:

- `gbf-hw/src/mbc5.rs`
- `gbf-hw/src/lcd.rs`
- `gbf-hw/src/timing.rs`
- `gbf-hw/src/interrupts.rs`
- `gbf-hw/src/joypad.rs`
- `gbf-hw/tests/cross_module_conformance.rs`
- `gbf-asm/src/effect.rs`

Mechanical or low-risk:

- `gbf-hw/src/lib.rs`
- `gbf-hw/Cargo.toml`
- `gbf-foundation/src/ids.rs`
- `gbf-foundation/src/lib.rs`
- `gbf-asm/src/rom.rs`

Review scaffolding:

- `gbf-hw/tests/single_source_smoke.rs`
- `gbf-hw/tests/single_source_smoke.allowlist.yaml`
- `scripts/lints/no-hw-literal-redeclarations.py`
- `docs/review/f-a2/**`
- `scripts/review/f-a2/verify-packet.sh`
