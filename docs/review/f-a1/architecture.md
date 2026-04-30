# Architecture

The executable path is strictly typed:

```text
Section
  lower_pre_layout_ops
LoweredSection
  layout_into_banks
LayoutPlan
  relax_and_legalize
LegalizedSection + final LayoutPlan + SymbolTable
  encode_section
EncodedSection
  assemble_rom / emit_listing / write_sym
```

`Section` owns labels, instructions, data blocks, alignments, pre-layout ops, legalization ops, and branches. `LoweredSection` drops pre-layout ops. `LegalizedSection` drops pre-layout ops, legalization ops, and branches. The encoder accepts only `LegalizedSection`, so unresolved symbolic branches and structured ops are absent at the type level.

ROM address mapping:

```text
ROM0 CPU $0000-$3FFF -> file offset $0000-$3FFF
ROMX bank N CPU $4000-$7FFF -> N * 0x4000 + (cpu - $4000)
WRAM/HRAM/SRAM/VRAM/OAM -> no ROM file offset
```
