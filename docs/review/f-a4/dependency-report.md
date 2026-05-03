# F-A4 Dependency Report

Runtime banking uses existing workspace crates only:

- `gbf-asm` for symbolic pre-layout wire records, builder APIs, lowering
  fragments, typed instructions, and section privilege metadata.
- `gbf-abi` for `InterruptPolicy`.
- `gbf-hw` for MBC5 and memory constants.
- `serde` / `serde_json` for host-side review and annotation shapes.

No new third-party dependency is introduced by F-A4.

