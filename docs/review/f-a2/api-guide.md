# API Guide

`cartridge_header`:

- Header-byte enums and `NINTENDO_LOGO`.
- `RomSize::{header_byte,bank_count,bytes,kib}` and `RamSize::{header_byte,kib,bank_count}`.

`target`:

- `TargetProfile`, `ConsoleModel`, `CartridgeProfile`, `CapabilitySet`.
- `dmg_mbc5_8mib_128kib()` is the M0 bring-up profile.

`memory`:

- Inclusive memory-region constants.
- `classify(u16) -> MemoryRegion`.
- Predicate helpers for ROM, SRAM, WRAM, HRAM, IO, ISR residency, and ISR I/O permission.

`mbc5`, `lcd`, `timing`, `interrupts`, `joypad`:

- Hardware constants and small pure predicates.

`calibration`:

- Constructor-validated `CycleDistribution` and `CalibrationConfidence`.
- Layered bundles and optional `CalibrationSetRef`.
- `MeasurementTarget` encodes emulator XOR hardware by type shape.
