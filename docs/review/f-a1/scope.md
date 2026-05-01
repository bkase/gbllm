# Scope

Implemented in F-A1:

- `cycle_model.rs`, `encoder.rs`, `layout.rs`, `relax.rs`, `lowering.rs`
- `listing.rs`
- `rom.rs`
- `.sym` writer additions in `symbols.rs`
- `gbf-asm/examples/tiny_rom.rs`
- deterministic `.gb`, `.lst`, and `.sym` generation
- structural ROM validation: header checksum, global checksum, bank packing, 0xFF fill
- typed lowering and legalization boundaries

Deferred, intentionally not claimed:

- live emulator boot validation: owned by follow-up `gbf-emu` / `gbf-debug`; F-A1 validates structural bytes and artifacts only
- production BankLease / BankGuard ABI: owned by F-A4
- F-A5 text renderer: `tiny_rom` uses a WRAM/HRAM-visible sentinel write and a loop
- Epic B reachability validation, hotness placement, bank-switch coalescing, stage cache integration, no-std migration, CGB/GBC support

F-A1 guard for deferred work: tests and packet scripts do not require an emulator, production bank-switch ABI, or text renderer.
