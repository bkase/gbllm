# F-A1 Review Packet

RFC: `history/rfcs/F-A1-gbf-asm.md`  
Branch: `f-a1-pr3-listing-rom-packet`  
PR stack:
- PR 1: cycle model + encoder
- PR 2: layout + relaxation + structured-op lowering
- PR 3: listing + `.sym` + ROM builder + `tiny_rom`

This packet covers the completed `gbf-asm` pipeline:

```text
Vec<Section>
  -> LoweredSection
  -> LayoutPlan
  -> LegalizedSection
  -> EncodedSection
  -> .gb / .lst / .sym
```

Highest-risk invariants: opcode correctness, type-state boundaries, branch relaxation fixed point, per-target far-call thunks, ROM header/checksum correctness, deterministic artifacts, and no raw-byte/unlegalized-op escape hatch.

Reviewer commands:

```bash
cargo test -p gbf-asm --all-features
cargo test --workspace --all-features
cargo run -p gbf-asm --example tiny_rom --features stub-runtime
./scripts/review/f-a1/verify-packet.sh
```

Expected artifacts:

```text
docs/review/f-a1/artifacts/tiny_rom.gb   32768 bytes
docs/review/f-a1/artifacts/tiny_rom.lst  deterministic listing
docs/review/f-a1/artifacts/tiny_rom.sym  deterministic RGBDS-compatible symbols
```

See `generated-artifacts.md` and `artifacts/tiny_rom.sha256` for hashes.

External review follow-up is summarized in `external-review-follow-up.md`.
