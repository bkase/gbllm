# F-A1 PR 2: Layout + Relaxation + Structured-Op Lowering

## Scope

This PR implements the placement-dependent middle of the F-A1 pipeline:

- `gbf-asm/src/lowering.rs`
- `gbf-asm/src/layout.rs`
- `gbf-asm/src/relax.rs`
- supporting `SectionRole`, `SymbolicBranch`, and `SymbolName` helpers

It is stacked on PR 1, which owns the cycle model and encoder. PR 3 owns the
listing emitter, `.sym` writer, ROM builder, `tiny_rom`, and the umbrella
reproducibility packet required by `history/rfcs/F-A1-review-packet-requirements.md`.

## Reviewer Order

1. Review `gbf-asm/src/section.rs` for the new data-only ROM roles and branch
   constructors.
2. Review `gbf-asm/src/lowering.rs` for the pre-layout lowering seam and stub
   policy behavior.
3. Review `gbf-asm/src/layout.rs` for bank/address placement, reserved ranges,
   concrete alignment padding, and JSON-safe bank keys.
4. Review `gbf-asm/src/relax.rs` for JR widening, direct reachability checks,
   explicit far-call thunk allocation, and final `LegalizedSection` output.

## Main Claims

| Claim | Implementation | Gate |
| --- | --- | --- |
| Pre-layout structured ops are physically drained before layout | `lower_pre_layout_ops` returns `LoweredSection` without `pre_layout_ops` | `lowering::pre_layout_ops_are_drained` |
| Stub lowering is explicit and policy-controlled | `StubLoweringConfig`, trace/assert elision modes | `lowering::elided_stub_ops_emit_no_items` |
| User-authored cartridge-header sections are rejected | `layout_into_banks` rejects `HeaderCartridge` | `layout` tests plus PR3 ROM header tests |
| ROM0/ROMX sections never cross their bank window | `PlacedSection::rom_file_offset`, `validate_placed` | `layout::no_section_crosses_bank`, `layout::rom_file_offset_rejects_bank_boundary_overflow` |
| Placement profiles are deterministic | BTreeMap-backed first-fit placement and stable section input order | `layout::strict_one_expert_per_bank_semantics`, `layout::budgeted_reserve_respected`, JSON round-trip test |
| Layout-chosen alignment padding is carried forward | `PlacedSection::alignment_padding` | encoder PR1 alignment-plan tests plus `relax::item_offsets` checks |
| Out-of-range symbolic JR widens to JP | `wide_jumps` fixed-point state in `relax_and_legalize` | `relax::out_of_range_jr_becomes_jp` |
| Cross-bank direct branches/calls are rejected | `ensure_directly_reachable` | `relax::cross_bank_jr_is_rejected`, `relax::plain_cross_bank_call_is_rejected` |
| Explicit far calls allocate one Bank-0 thunk per target symbol | `ResolvedThunkRequest`, `runtime_thunk_for`, thunk dedup map | `relax::explicit_far_call_becomes_per_target_thunk`, `relax::two_callsites_share_one_thunk` |
| Encoder receives only legalized arrays | `RelaxedProgram.sections: Vec<LegalizedSection>` | `cargo test -p gbf-asm` type-checks the pipeline |

## Exact Commands Run

```bash
cargo test -p gbf-asm
```

Latest local result:

```text
cargo test -p gbf-asm
52 passed; 0 failed; 0 ignored
```

## Deferred To PR 3

- `.lst` emission and listing-stability tests.
- RGBDS-compatible `.sym` output.
- Cartridge header construction, checksum validation, and ROM packing.
- `examples/tiny_rom.rs`.
- `scripts/review/f-a1/verify-packet.sh` and checked generated artifacts.
