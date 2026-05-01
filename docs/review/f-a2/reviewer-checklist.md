# Reviewer Checklist

- [ ] `gbf-asm::rom` only re-exports cartridge-header facts; `CartridgeHeader` remains in `gbf-asm`.
- [ ] Every `u16` address maps to exactly one `MemoryRegion`.
- [ ] DMG and CGB ISR residency predicates differ at `WRAMX`.
- [ ] `is_isr_io_register_allowed` is separate from residency.
- [ ] MBC5 RAM-enable exposes canonical `$0A` only.
- [ ] `PpuMode::from_stat_bits` is the integer-to-mode decode path.
- [ ] Interrupt sources are in hardware priority order.
- [ ] `ButtonState` is active-high and separate from active-low JOYP bits.
- [ ] Calibration constructors reject impossible confidence/distribution states.
- [ ] `serde_json` is dev-only and `chrono` is absent.
