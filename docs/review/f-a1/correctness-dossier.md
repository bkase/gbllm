# Correctness Dossier

Type-state proof: `Section` has unresolved arrays, `LoweredSection` removes `pre_layout_ops`, and `LegalizedSection` removes `pre_layout_ops`, `legalization_ops`, and `branches`. `encode_section` accepts only `&LegalizedSection`.

Unique byte-emission proof:

- `Instr` bytes are produced only by `encode_instr`.
- `DataBlock::Bytes` and `DataBlock::Words` are lowered by `encode_section`.
- `Align` bytes come from `PlacedSection::alignment_padding` and use `PAD_BYTE`.
- The cartridge header is built as an internal `LegalizedSection` and encoded through `encode_section`.
- There is no `Raw` item, no `MachineEffect::OpaqueBytes`, and layout rejects user `HeaderCartridge` sections.

ROM proof: `rom::assemble_rom` validates header fields, rejects non-ROM sections and header-range collisions, maps ROMX file offsets with `bank * 0x4000 + cpu - 0x4000`, fills unused bytes with `0xFF`, and writes header/global checksums using Pan Docs algorithms.

Listing and `.sym` proof: listings are derived from encoded item spans and section placement. `.sym` entries are sorted by location and name; dot-safe names use the injective `.` -> `_d`, `_` -> `__` escape with a collision check.
