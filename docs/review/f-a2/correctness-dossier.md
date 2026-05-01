# Correctness Dossier

Cartridge header:

- Invariant: F-A1 public names and serde spelling remain stable.
- Gate: `cartridge_header::*` unit tests and `cargo test -p gbf-asm`.

Memory map:

- Invariant: every `u16` classifies into exactly one `MemoryRegion`.
- Gate: `memory::region_classification` and `memory::no_predicate_overlap`.

ISR safety:

- Invariant: residency and I/O permission are separate predicates.
- Gate: `memory::isr_resident_legal_dmg`, `memory::isr_resident_legal_cgb`, and `memory::isr_io_register_allowed`. `IE` is both the singleton `InterruptEnable` residency byte and an allowed ISR I/O register.

MBC5:

- Invariant: `$0A` is the only canonical RAM-enable value exposed, and `$6000..=$7FFF` is named `Reserved`.
- Gate: `mbc5::ram_enable_value`, `mbc5::reserved_band_is_named`, and `mbc5::loose_ram_enable_not_provided`.

LCD/timing:

- Invariant: mode bits decode to `PpuMode`, VRAM/OAM accessibility matches Pan Docs, and frame/VBlank M-cycle math is fixed.
- Gate: `lcd::*` and `timing::*` tests.

Interrupts:

- Invariant: vectors, IE/IF bits, and priority order match LR35902 hardware.
- Gate: `interrupts::*` tests and `cross_module_conformance::interrupt_vectors_live_in_bank0`.

Calibration:

- Invariant: raw confidence and cycle distributions cannot be forged through JSON.
- Gate: `calibration::serde_runs_validation` plus constructor negative tests.
