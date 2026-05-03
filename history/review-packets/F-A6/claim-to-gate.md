# F-A6 Claim-To-Gate Matrix

| Claim | Gate |
| --- | --- |
| BlobRef has hash/len/codec and serde-stable codec names | gbf-foundation/tests/blob.rs::blob_ref_serde_round_trip + blob_codec_serde_snake_case |
| gbf-store has no local unsafe | gbf-store/src/lib.rs contains #![forbid(unsafe_code)] and cargo clippy passes |
| BlobStore writes are content-addressed and idempotent | blob::round_trip + blob::content_addressed_idempotent |
| put_expect rejects mismatched claimed hashes before commit | blob::put_expect_hash_mismatch + archive::extract_rejects_record_exceeding_declared_total_before_commit |
| Canonical existing files are rehashed before idempotent reuse | blob::idempotent_verifies_existing |
| BlobRef length is validated on read | blob::get_ref_validates_len |
| Integrity check detects missing and corrupt blobs | integrity::verify_missing_blob + integrity::verify_corrupt_blob |
| StageCache compose_key is deterministic across construction order | stage_cache::deterministic_keys |
| StageCache key encoding is boundary-safe and SemVer checked | stage_cache::compose_key_length_prefix_safe + pass_version_component_overflow_rejected |
| StageCache stale payloads are cache misses | stage_cache::stale_index_treated_as_miss |
| Pinset names reject path-shaped names | pinset::name_validation_rejects_bad_forms + name_validation_rejects_leading_dot |
| GC protects pinsets and walks transitive references | gc::pinset_protection + gc::transitive_refs_via_registry |
| GC default unknown-reference policy aborts | gc::unknown_reference_policy_abort |
| GC dry-run and removal limits are deterministic | gc::dry_run_populates_candidates + gc::max_remove_per_run_honored + removal_order_is_hash_ascending |
| GC sweeps only indexes for blobs removed in the run | gc::sweep_stage_cache_indexes_removes_stale + sweep_stage_cache_indexes_respects_remove_limit |
| Archive header is byte-stable | archive::magic_bytes + version_byte + header_size_24_bytes |
| Archive bytes are deterministic for equivalent inputs | archive::deterministic_bytes + blobs_sorted_by_hash + pinsets_sorted_by_name_internally |
| Archive extraction is hash-checked and non-transactional | archive::extract_validates_each_record_hash + extract_not_transactional |
| Archive rejects malformed accounting before commit | archive::extract_rejects_record_exceeding_declared_total_before_commit + total_bytes_mismatch_rejected + trailing_bytes_rejected |
| gbf-store Cargo cleanup removes gbf-artifact | cargo tree -p gbf-store --depth 1 in dependency-report.md |
| gbf-migrate is untouched and deferred | git diff --exit-code -- gbf-migrate + cargo test -p gbf-migrate |
