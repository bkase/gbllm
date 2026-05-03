# F-A4 Claim To Gate

| Claim | Gate |
|---|---|
| ROM bank 0 is rejected by ABI policy | `banking::lease_spec_invariants_rom` |
| SRAM bank range is validated | `banking::lease_spec_invariants_sram` |
| `BankGuard` does not borrow `Builder` | `banking::bank_guard_does_not_borrow_builder` |
| Unreleased guards fail `try_finish` | `banking::bank_guard_drop_without_release` |
| Stale/double release is typed | `banking::bank_guard_double_release` |
| Cross-builder guard reuse is stale | `banking::bank_guard_from_other_builder_is_stale` |
| Manual leases cannot yield while held | `banking::manual_lease_rejects_yield_while_held` |
| Durable lease lifetime is preserved into lowering | `banking::lowering_preserves_manual_lifetime_from_wire_spec` |
| ResumeWindow/Token reject until scheduler restoration exists | `banking::lowering_rejects_restoration_lifetimes_until_scheduler_owner_lands` |
| HRAM banking shadow owns exactly `$FF80..=$FF83` | `banking::hram_shadow_offsets_within_banking_range`, `banking::hram_banking_shadow_region_size` |
| Shadow zero init is 10 bytes / 14 M-cycles | `banking::banking_shadow_zero_init_byte_and_cycle_count` |
| ROM acquire SCS sequence is byte-stable | `banking::lower_acquire_rom_bank_byte_sequence_bank3`, `banking::lower_acquire_rom_bank_byte_sequence_bank256` |
| Disabled policy removes DI/EI | `banking::lower_acquire_rom_bank_disabled_policy` |
| Enabled policy is rejected for acquire | `banking::lower_acquire_rom_bank_rejects_enabled_policy` |
| SRAM acquire requires SRAM enabled | `banking::lower_acquire_sram_bank_rejects_disabled` |
| Release to Bank1 reuses acquire path | `banking::lower_release_to_bank1_is_acquire_one` |
| KeepCurrent emits no instructions and needs trusted lowering provenance | `banking::lower_release_keep_current_is_label_only`, `banking::composite_lowerer_does_not_swallow_error` |
| Normal/ISR/switchable sections cannot emit banking writes | `banking::normal_section_cannot_lease`, `banking::isr_section_cannot_lease`, `banking::switchable_residency_cannot_lease` |
| Banking `PreLayoutOp` lowering emits acquire/release fragments | `banking::lowering_bank_lease_emits_acquire_sequence`, `banking::lowering_bank_release_per_return_state` |
| `AssertBank` compare-and-trap profile emits concrete check bytes | `banking::lowering_assert_bank_emits_compare_and_trap_when_enabled` |
| `AssertBank` compare-and-trap rejects invalid ROM banks | `banking::lowering_assert_bank_compare_rejects_invalid_rom_bank` |
| Non-banking ops return `NotOwned` | `banking::lowering_rejects_non_banking_ops` |
| Composite lowerer does not swallow owned errors | `banking::composite_lowerer_does_not_swallow_error` |
| MBC writes have banking provenance | `banking::mbc_write_provenance_audit_catches_non_banking_source`, `banking::mbc_write_provenance_audit_rejects_forged_public_source_string`, `banking::mbc_write_provenance_audit_rejects_forged_public_source_and_note`, `banking::full_acquire_release_round_trip` |
