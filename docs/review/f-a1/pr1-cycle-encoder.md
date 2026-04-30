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

1. Read `gbf-asm/src/isa.rs` enough to see the concrete `Instr` variants and
   `Instr::byte_len`.
2. Review `gbf-asm/src/cycle_model.rs`.
3. Review `gbf-asm/src/encoder.rs`.
4. Skim `gbf-asm/src/layout.rs` for the `PlacedSection` / `AddressSpace`
   boundary consumed by `encode_section`.

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

Codex, Gemini, and Claude review passes were requested for this PR. Accepted
findings applied here:

- `LDH A,(C)` / `LDH (C),A` cycle costs are 2 M-cycles.
- ROM file offsets now reject sections that cross ROM0/ROMX bank boundaries.
- `encode_section` rejects section/placement mismatches and invalid placement
  tuples before emitting bytes.
- Alignment plans now reject both missing and extra padding entries.
- Span/offset conversions fail explicitly instead of truncating.

## Notes For Follow-Up PRs

PR2 owns the full interval allocator, lowering seams, branch relaxation, and
far-call thunk materialization. PR3 owns listing, `.sym`, ROM assembly,
`tiny_rom`, and the umbrella reproducibility packet.
