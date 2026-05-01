# Diff Map

| File | Change | Risk | Main gates |
| --- | --- | --- | --- |
| `gbf-asm/src/cycle_model.rs` | PR1 implementation | Medium | `cycle_model::known_instructions`, `t_states_lossless` |
| `gbf-asm/src/encoder.rs` | PR1 + PR3 span-kind ordering | High | `encoder::known_opcodes`, `cb_prefix_table_is_exhaustive`, `encode_instr_matches_byte_len` |
| `gbf-asm/src/layout.rs` | PR2 implementation | High | `layout::no_section_crosses_bank`, `bank0_auto_placement_skips_pinned_sections` |
| `gbf-asm/src/relax.rs` | PR2 implementation | High | `relax::out_of_range_jr_becomes_jp`, `auto_far_symbolic_call_becomes_per_target_thunk` |
| `gbf-asm/src/lowering.rs` | PR2 implementation | High | `lowering::pre_layout_ops_are_drained`, `lowered_fragments_preserve_sub_index_order` |
| `gbf-asm/src/listing.rs` | PR3 implementation | Medium | `listing::byte_stable`, `format_instr_canonical`, `large_data_block_is_chunked_deterministically` |
| `gbf-asm/src/rom.rs` | PR3 implementation | High | `rom::header_checksum_known_vector`, `global_checksum_round_trip`, `bank_n_at_correct_offset` |
| `gbf-asm/src/symbols.rs` | PR3 `.sym` writer/parser | Medium | `symbols::write_sym_sorted`, `write_sym_dot_safe_escape_avoids_naive_collision` |
| `gbf-asm/examples/tiny_rom.rs` | PR3 end-to-end example | Medium | `cargo run -p gbf-asm --example tiny_rom --features stub-runtime` |
| `scripts/review/f-a1/*` | packet build/verify scripts | Medium | `./scripts/review/f-a1/verify-packet.sh` |
