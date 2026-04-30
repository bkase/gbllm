# F-A1 PR 1: Cycle Model + Encoder

## Scope

This PR implements the deterministic instruction-level parts of F-A1:

- `gbf-asm/src/cycle_model.rs`
- `gbf-asm/src/encoder.rs`
- shared placement facts in `gbf-asm/src/layout.rs` needed by the encoder API

It also carries the F-A1 RFC alignment work already present on the branch:
typed SoA section state, `LoweredSection`, `LegalizedSection`, and removal of
the raw-byte escape hatch from the design.

## Reviewer Order

1. Use `gbf-asm/src/isa.rs` as base-tree context only. It defines the concrete
   `Instr` variants and `Instr::byte_len`, but it is not part of this PR diff.
2. Review `gbf-asm/src/section.rs` for the typed SoA section IR,
   `LoweredSection` / `LegalizedSection`, and the removed raw-byte escape
   hatch.
3. Review `gbf-asm/src/builder.rs` for the public builder surface, section
   role checks, and executable-section inline-data rejection.
4. Review `gbf-asm/src/effect.rs` for privilege classification changes,
   especially reserved MBC writes and structured-op privilege.
5. Review `gbf-asm/src/cycle_model.rs`.
6. Skim the changed pieces of `gbf-asm/src/layout.rs` for the
   `PlacedSection` / `AddressSpace` boundary consumed by `encode_section`.
7. Review `gbf-asm/src/encoder.rs`.

## Changed File Disposition

This is the complete changed-file set from the GitHub PR diff.

| File | Reviewer handling |
| --- | --- |
| `.beads/issues.jsonl` | Skip line review; issue-tracker export only. Check only if you want bead closure provenance. |
| `docs/review/f-a1/pr1-cycle-encoder.md` | Read first; this packet explains how to review the PR. |
| `gbf-asm/src/builder.rs` | Review focused on builder APIs, SoA emission, privilege checks, and inline-data rejection. |
| `gbf-asm/src/cycle_model.rs` | Deep review; this is one of PR1's primary implementation files. |
| `gbf-asm/src/effect.rs` | Focused review of privilege classification, reserved MBC register handling, and structured-op effects. |
| `gbf-asm/src/encoder.rs` | Deep review; this is one of PR1's primary implementation files. |
| `gbf-asm/src/layout.rs` | Focused review of shared placement facts and ROM offset validation. PR2 owns the full allocator. |
| `gbf-asm/src/section.rs` | Deep review; this owns the symbolic pre-layout IR and the type-state boundary used by the encoder. |
| `history/glossary.md` | Skim for terminology alignment only. |
| `history/planv0.md` | Skim for F-A1 scope alignment only; not executable code. |
| `history/rfcs/F-A1-gbf-asm.md` | Review as the implementation contract for this stack, especially PR1-owned claims. |
| `history/rfcs/F-A1-review-packet-requirements.md` | Skim as the packet contract; use it to judge whether the review packet is sufficient. |

## Main Claims

| Claim | Implementation | Gate |
| --- | --- | --- |
| Every `Instr` has a nonzero M-cycle cost | `cycle_model::cycle_cost` | `cargo test -p gbf-asm cycle_model::` |
| Conditional branch timing is family-specific | `CycleCost::Branch` arms | `conditional_branch_timings_by_family` |
| M-cycle to T-state conversion is lossless | `CycleCost::t_states` | `t_states_lossless` |
| The encoder is the only `Instr -> bytes` path | `encoder::encode_instr` | `known_opcodes`, `encode_instr_matches_byte_len` |
| CB-prefixed opcodes are exhaustively checked | `encode_cb` + table test | `cb_prefix_table_is_exhaustive` |
| `encode_section` cannot receive structured ops or unresolved branches | API accepts `LegalizedSection` only | `section::legalized_section_drops_unencoded_arrays_at_the_type_level` |
| Alignment bytes come from layout, not recomputation | `PlacedSection::alignment_padding` | `encode_section_merges_legalized_arrays_in_order` |

## Exact Commands Run

```bash
cargo test -p gbf-asm cycle_model::
cargo test -p gbf-asm encoder::
cargo test -p gbf-asm
```

Latest local result:

```text
cargo test -p gbf-asm
40 passed; 0 failed; 0 ignored
```

## External Review Follow-Up

Gemini and Claude both returned actionable PR1 findings. The table below is
the disposition of those reviews; it is not another list of files to inspect.

| Review finding | Disposition |
| --- | --- |
| `LDH A,(C)` / `LDH (C),A` cycle costs were 1 M-cycle instead of 2 | Accepted; `cycle_model::known_instructions` pins both at 2 M-cycles. |
| ROM file offsets did not reject sections crossing ROM0/ROMX bank boundaries | Accepted; `PlacedSection::rom_file_offset` validates full-section bank windows. |
| `encode_section` used an overloaded size-mismatch error for large offsets | Accepted; offset overflow is a distinct `EncodeError::SectionOffsetOverflow`. |
| `EncodedItemSpan::len` could truncate materialized byte lengths | Accepted; span lengths now use checked conversion through `checked_span_len`. |
| Extra `alignment_padding` entries were ignored | Accepted; stale alignment plans now return `EncodeError::ExtraAlignmentPlan`. |
| Padding byte `0xFF` was unnamed | Accepted; `encoder::PAD_BYTE` names the value and documents the rationale. |
| Encoder error paths lacked coverage | Accepted for PR1-owned errors: missing/extra alignment, non-ROM placement, bad placement tuple, offset overflow, and size mismatch are covered. |
| `HeaderCartridge` permits inline data | Deferred to the stack boundary: PR2 rejects user-authored header sections, and PR3 owns the internal ROM header overlay. |
| `LegalizedSection` compile-fail proof was requested | Not added in PR1; the packet no longer claims a compile-fail doctest, only the implemented type shape and JSON-shape regression. |
| Label items do not produce `EncodedItemSpan` entries | Intentional; labels are zero-byte symbolic positions, and PR3's symbol/listing path resolves addresses from the symbol table rather than item spans. |
| `Pop` grouping in `cycle_model.rs` was stylistic | Left unchanged; the tested cost is correct. |
| `encode_data_block` could return a length directly | Left unchanged; the caller validates the materialized span length and final section size after emission. |

## Notes For Follow-Up PRs

PR2 owns the full interval allocator, lowering seams, branch relaxation, and
far-call thunk materialization. PR3 owns listing, `.sym`, ROM assembly,
`tiny_rom`, and the umbrella reproducibility packet.
