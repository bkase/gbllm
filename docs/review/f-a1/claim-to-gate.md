# Claim To Gate

| Claim | Gate |
| --- | --- |
| Cycle costs match Pan Docs spot checks | `cycle_model::known_instructions` |
| No zero cycle costs | `cycle_model::no_zero_cost` |
| Instruction byte lengths are pinned | `isa::instr_size_in_bytes` |
| Encoder matches gbdev opcode tables | `encoder::tests::unprefixed_opcodes_match_gbdev_json`, `encoder::tests::cb_prefixed_opcodes_match_gbdev_json` |
| Layout respects ROM/header/thunk intervals | `layout::tests::bank0_auto_placement_skips_pinned_sections`, `layout::tests::pinned_placements_cannot_overlap` |
| Relaxation widens out-of-range JR and rejects cross-bank JR | `relax::tests::out_of_range_jr_becomes_jp`, `relax::tests::cross_bank_jr_is_rejected` |
| AutoFar calls allocate per-target thunks | `relax::tests::auto_far_symbolic_call_becomes_per_target_thunk`, `relax::tests::two_callsites_share_one_thunk` |
| Listing is byte-stable and option-sensitive | `listing::tests::byte_stable`, `listing::tests::all_options_render` |
| Listing fails closed on malformed encoded spans | `listing::tests::missing_encoded_span_is_error`, `listing::tests::extra_encoded_span_is_error`, `listing::tests::out_of_bounds_encoded_span_is_error` |
| Program listings are emitted in placed ROM order | `listing::tests::program_listing_orders_sections_by_placed_rom_offset` |
| ROM header/checksum/padding is structural | `rom::tests::header_checksum_known_vector`, `rom::tests::global_checksum_round_trip`, `rom::tests::unused_regions_are_ff` |
| ROM assembly rejects malformed section/package inputs | `rom::tests::overlapping_sections_are_rejected`, `rom::tests::section_size_mismatch_is_rejected`, `rom::tests::entry_point_is_required` |
| `.sym` is sorted and dot-safe escaping is injective | `symbols::sym_tests::write_sym_sorted`, `symbols::sym_tests::write_sym_dot_safe_escape_avoids_naive_collision` |
| Tiny ROM artifacts are reproducible | `./scripts/review/f-a1/verify-packet.sh` |
