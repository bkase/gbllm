# Claim To Gate

| Claim | Gate |
| --- | --- |
| Cycle costs match Pan Docs spot checks | `cycle_model::known_instructions` |
| No zero cycle costs | `cycle_model::no_zero_cost` |
| Encoder matches `Instr::byte_len` | `encoder::encode_instr_matches_byte_len` |
| CB-prefix encoding is exhaustive | `encoder::cb_prefix_table_is_exhaustive` |
| Layout respects ROM/header/thunk intervals | `layout::bank0_auto_placement_skips_pinned_sections`, `layout::pinned_placements_cannot_overlap` |
| Relaxation widens out-of-range JR and rejects cross-bank JR | `relax::out_of_range_jr_becomes_jp`, `relax::cross_bank_jr_is_rejected` |
| AutoFar calls allocate per-target thunks | `relax::auto_far_symbolic_call_becomes_per_target_thunk`, `relax::two_callsites_share_one_thunk` |
| Listing is byte-stable and option-sensitive | `listing::byte_stable`, `listing::all_options_render` |
| ROM header/checksum/padding is structural | `rom::header_checksum_known_vector`, `rom::global_checksum_round_trip`, `rom::unused_regions_are_ff` |
| `.sym` is sorted and dot-safe escaping is injective | `symbols::write_sym_sorted`, `symbols::write_sym_dot_safe_escape_avoids_naive_collision` |
| Tiny ROM artifacts are reproducible | `./scripts/review/f-a1/verify-packet.sh` |
