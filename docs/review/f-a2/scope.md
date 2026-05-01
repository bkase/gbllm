# Scope

In scope:

- `gbf-hw::cartridge_header` with F-A1-compatible `NINTENDO_LOGO`, `MbcType`, `RomSize`, `RamSize`, and `DestinationCode`.
- `gbf-asm::rom` re-export migration, leaving the composite `CartridgeHeader` builder in `gbf-asm`.
- `gbf-asm::effect` reuse of `gbf-hw` MBC5 and memory classifiers, including Echo RAM / unmapped memory as `Unusable`.
- `gbf-hw::{target,memory,mbc5,lcd,timing,interrupts,joypad,calibration}`.
- Additive foundation ID newtypes for calibration layers.
- Focused tests, cross-module conformance checks, and the ignored single-source smoke test.

Out of scope:

- Calibration production (`gbf-bench` / Epic E).
- BankLease and runtime bank shadow state (F-A4).
- Runtime joypad reader and video commit implementation (F-A5).
- Whole-program reachability validation (Epic B).
- Switching `gbf-foundation` and `gbf-hw` to declared `no_std + alloc`.
