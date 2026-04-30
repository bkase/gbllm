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

## Base Context

Review this PR against PR 1, not against `main`. PR 1 owns the cycle model,
the canonical opcode encoder, the initial SoA section split, effect typing, and
the no-raw-byte builder boundary. PR 2 only changes those surfaces where layout,
lowering, relaxation, or thunking need additional metadata.

## Reviewer Order

1. Use `docs/review/f-a1/pr1-cycle-encoder.md` as base context for the SoA,
   privilege, cycle, and encoder contracts already reviewed in PR 1.
2. Review `gbf-asm/src/section.rs` for the new data-only ROM roles,
   `ItemOrder`, `CallReachability`, and privilege retention across
   `LoweredSection` / `LegalizedSection`.
3. Review `gbf-asm/src/symbols.rs` for deterministic thunk naming and symbol
   table iteration helpers.
4. Review `gbf-asm/src/lowering.rs` for the pre-layout lowering seam, fragment
   sub-ordering, fragment privilege revalidation, and stub policy behavior.
5. Review `gbf-asm/src/layout.rs` for bank/address placement, reserved ranges,
   occupied-interval collision checks, concrete alignment padding, and
   JSON-safe bank keys.
6. Review `gbf-asm/src/relax.rs` for JR widening, direct reachability checks,
   explicit far-call thunk allocation, thunk-pool capacity, and final
   `LegalizedSection` output.
7. Review the changed pieces of `gbf-asm/src/encoder.rs` for `ItemOrder`
   ordering, `sub_index` spans, and alignment-plan keying.
8. Skim `gbf-asm/src/builder.rs` and `gbf-asm/src/lib.rs`; they only expose
   PR2-owned roles/modules or extend tests for those roles.

## Changed File Disposition

This is the complete changed-file set from the GitHub PR diff.

| File | Reviewer handling |
| --- | --- |
| `.beads/issues.jsonl` | Skip line review; issue-tracker export only. Check only if you want bead closure provenance. |
| `docs/review/f-a1/pr2-layout-relax-lowering.md` | Read first; this packet explains how to review the PR. |
| `gbf-asm/src/builder.rs` | Skim the test update for data-only ROM roles. Builder API behavior is otherwise PR1-owned. |
| `gbf-asm/src/encoder.rs` | Focused review of `ItemOrder` / `sub_index` span ordering and alignment-plan key changes. Opcode byte encoding is PR1-owned. |
| `gbf-asm/src/layout.rs` | Deep review; this is one of PR2's primary implementation files. |
| `gbf-asm/src/lib.rs` | Skim; it exposes the new `lowering` module. |
| `gbf-asm/src/lowering.rs` | Deep review; this is one of PR2's primary implementation files. |
| `gbf-asm/src/relax.rs` | Deep review; this is one of PR2's primary implementation files. |
| `gbf-asm/src/section.rs` | Focused review of PR2-added roles, `ItemOrder`, `CallReachability`, and stage privilege fields. |
| `gbf-asm/src/symbols.rs` | Focused review of thunk symbol naming and symbol-table iteration. |

## Main Claims

| Claim | Implementation | Gate |
| --- | --- | --- |
| Pre-layout structured ops are physically drained before layout | `lower_pre_layout_ops` returns `LoweredSection` without `pre_layout_ops` | `lowering::pre_layout_ops_are_drained` |
| Lowering fragments cannot bypass section privilege | `validate_fragment` checks lowered instructions, branches, and late ops before splice | `lowering::lowered_fragments_are_revalidated_against_section_privilege` |
| Lowered multi-item fragments keep local order | `ItemOrder { seq_index, sub_index }` | `lowering::lowered_fragments_preserve_sub_index_order` |
| Stub lowering is explicit and policy-controlled | `StubLoweringConfig`, trace/assert elision modes | `lowering::elided_stub_ops_emit_no_items` |
| User-authored cartridge-header sections are rejected | `layout_into_banks` rejects `HeaderCartridge` | `layout::user_header_section_rejected` |
| ROM0 reserves the header/vectors and high thunk pool | `ReservedRange` + occupied-interval placement | `layout::bank0_auto_placement_skips_pinned_sections`, `layout::pinned_placements_cannot_overlap` |
| ROM0/ROMX sections never cross their bank window | `PlacedSection::rom_file_offset`, `validate_placed` | `layout::no_section_crosses_bank`, `layout::rom_file_offset_rejects_bank_boundary_overflow` |
| Placement profiles are deterministic | BTreeMap-backed first-fit placement and stable section input order | `layout::strict_one_expert_per_bank_semantics`, `layout::strict_expert_banks_skip_common_banks`, `layout::budgeted_reserve_respected`, JSON round-trip test |
| Layout-chosen alignment padding is carried forward and recomputed after JR decisions | `PlacedSection::alignment_padding` keyed by `ItemOrder` | encoder PR1 alignment-plan tests plus `relax::short_jr_recomputes_alignment_padding` |
| Out-of-range symbolic JR widens to JP | `wide_jumps` fixed-point state in `relax_and_legalize` | `relax::out_of_range_jr_becomes_jp` |
| Cross-bank direct branches/calls are rejected | `ensure_directly_reachable` | `relax::cross_bank_jr_is_rejected`, `relax::plain_cross_bank_call_is_rejected` |
| Explicit far calls allocate one Bank-0 thunk per target symbol | `ResolvedThunkRequest`, `runtime_thunk_for`, thunk dedup map | `relax::explicit_far_call_becomes_per_target_thunk`, `relax::two_callsites_share_one_thunk`, `relax::thunk_pool_overflow_is_reported` |
| Symbolic calls distinguish near-only from auto-far intent | `CallReachability::{NearOnly, AutoFar}` | `relax::plain_cross_bank_call_is_rejected`, `relax::auto_far_symbolic_call_becomes_per_target_thunk` |
| Encoder receives only legalized arrays | `RelaxedProgram.sections: Vec<LegalizedSection>` | `cargo test -p gbf-asm` type-checks the pipeline |

## Exact Commands Run

```bash
cargo test -p gbf-asm
```

Latest local result:

```text
cargo test -p gbf-asm
59 passed; 0 failed; 0 ignored
```

Additional local gate:

```bash
cargo clippy -p gbf-asm --all-features -- -D warnings
```

## External Review Follow-Up

Gemini and Codex review passes returned actionable PR2 findings. The table
below is the disposition of those reviews; it is not another list of files to
inspect.

| Review finding | Disposition |
| --- | --- |
| Lowered fragments with multiple items lost authoring order | Accepted; `ItemOrder { seq_index, sub_index }` is carried through lowering, relax, encoder spans, and alignment plans. |
| Lowered fragments could bypass section privilege | Accepted; lowered instrs, branches, and late ops are revalidated against `SectionPrivilege` before splice. |
| `LoweredSection` / `LegalizedSection` dropped privilege metadata | Accepted; both stage-transition structs now retain `SectionPrivilege`. |
| Pinned Bank0 sections could reset placement into vectors/header space | Accepted; pinned and reserved ranges are tracked as occupied intervals. |
| Bank0 code could collide with far-call thunks | Accepted; `$3F00..=$3FFF` is reserved as the thunk pool. |
| Thunk slots could overflow out of Bank0 | Accepted; overflow returns `RelaxError::ThunkPoolExhausted`. |
| Relax did not recompute alignment after final JR widths | Accepted; relax recomputes item offsets and alignment padding after width decisions. |
| Strict expert placement could reuse a common bank | Accepted; strict expert placement skips banks already occupied by common sections. |
| Near calls and explicit auto-far calls were not distinguished | Accepted; `CallReachability::{NearOnly, AutoFar}` records author intent. |
| Far-call lease chains were dropped | Accepted; `ResolvedThunkRequest` preserves `lease_chain`. |
| User-authored `HeaderCartridge` sections need a clear owner | Accepted for PR2; layout rejects user-authored header sections. PR3 owns the internal ROM header overlay. |
| Stub runtime call targets are only placeholders | Deferred; PR2 keeps stub lowering explicit, but production runtime materialization/linking is outside this PR. |
| Full `ThunkMaterializer` / production banking ABI was requested | Deferred to the runtime/banking ABI work; PR2 owns deterministic request/thunk allocation, not final F-A4 ABI bytes. |
| Additional RFC matrix tests were requested beyond current gates | Partially accepted; packet claims now cite only tests that exist in this stack, with remaining ROM/listing/artifact gates deferred to PR3. |

## Deferred To PR 3

- `.lst` emission and listing-stability tests.
- RGBDS-compatible `.sym` output.
- Cartridge header construction, checksum validation, and ROM packing.
- `examples/tiny_rom.rs`.
- `scripts/review/f-a1/verify-packet.sh` and checked generated artifacts.
