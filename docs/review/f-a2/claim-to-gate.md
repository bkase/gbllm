# Claim To Gate

| Claim | Gate |
|---|---|
| F-A1 cartridge constants moved without API break | `cargo test -p gbf-asm` |
| `gbf-asm` effect classification uses `gbf-hw` memory/MBC5 facts | `cargo test -p gbf-asm -- effect::static_memory_boundaries_follow_gbf_hw`, `cargo test -p gbf-asm -- effect::mbc_writes_are_privileged` |
| `serde_json` is dev-only for `gbf-hw` | `cargo tree -p gbf-hw` and `gbf-hw/Cargo.toml` |
| Unsafe is forbidden | `#![forbid(unsafe_code)]` in `gbf-hw/src/lib.rs` |
| Every address classifies exactly once | `memory::region_classification`, `memory::no_predicate_overlap` |
| WRAM split differs between DMG and CGB | `memory::wram_split_dmg_vs_cgb` |
| ISR residency differs from ISR I/O permission | `memory::isr_resident_legal_*`, `memory::isr_io_register_allowed` |
| MBC5 BANK1/BANK2 form a 9-bit bank number | `mbc5::bank_number_assembly` |
| Loose MBC5 RAM-enable helper is absent | `mbc5::loose_ram_enable_not_provided` |
| LCD-disabled state allows VRAM/OAM access | `lcd::lcd_disabled_unrestricted` |
| Frame and VBlank M-cycle counts match Pan Docs | `timing::frame_cycles`, `timing::vblank_cycles` |
| Interrupt priority order is fixed | `interrupts::priority_order` |
| JOYP post-decode state is active-high | `joypad::is_pressed_table_driven` |
| Calibration confidence class is derived | `calibration::confidence_class_is_derived` |
| Kernel calibration rejects empty profiles | `calibration::kernel_bundle_rejects_empty_profiles` |
| Serde runs validation | `calibration::serde_runs_validation` |
| Cross-module constants agree | `gbf-hw/tests/cross_module_conformance.rs` |
