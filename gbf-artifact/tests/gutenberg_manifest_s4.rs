use gbf_artifact::{
    CanonicalGutenbergManifestWrite, GutenbergCompressionKind, GutenbergDedupPolicy,
    GutenbergDropReason, GutenbergFetchNamespaceKind, GutenbergManifest, GutenbergManifestError,
    GutenbergSourceRecord, GutenbergSplit,
};
use gbf_foundation::Hash256;

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn retained_source(book_id: u32, split: GutenbergSplit) -> GutenbergSourceRecord {
    GutenbergSourceRecord {
        book_id,
        title: format!("Book {book_id}"),
        author: "Anonymous".to_owned(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{book_id}"),
        mirror_fetch_url: Some(format!("file://mirror/{book_id}.txt")),
        mirror_snapshot_id: Some("fixture-2026-05-19".to_owned()),
        selected_format: Some("text/plain;charset=utf-8".to_owned()),
        source_blob_sha256: Some(hash(0x10 + book_id as u8)),
        pre_strip_utf8_sha256: Some(hash(0x20 + book_id as u8)),
        license: GutenbergSourceRecord::public_domain_in_usa_license(),
        fetch_namespace_kind: Some(GutenbergFetchNamespaceKind::ContentAddressedCache),
        fetch_namespace_id: Some("fixtures/corpora/gutenberg.toml".to_owned()),
        compression_kind: Some(GutenbergCompressionKind::None),
        archive_member_path: None,
        pre_strip_byte_length: Some(128 + u64::from(book_id)),
        drop_reason: None,
        duplicate_of_book_id: None,
        post_strip_byte_length: Some(96 + u64::from(book_id)),
        post_strip_sha256: Some(hash(0x30 + book_id as u8)),
        post_charset_body_sha256: Some(hash(0x40 + book_id as u8)),
        post_charset_token_length: Some(80 + u64::from(book_id)),
        unmappable_count: Some(0),
        unmappable_density: Some(0.0),
        split: Some(split),
    }
}

fn dropped_source(book_id: u32, reason: GutenbergDropReason) -> GutenbergSourceRecord {
    GutenbergSourceRecord {
        book_id,
        title: format!("Dropped {book_id}"),
        author: "Anonymous".to_owned(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{book_id}"),
        mirror_fetch_url: None,
        mirror_snapshot_id: None,
        selected_format: None,
        source_blob_sha256: None,
        pre_strip_utf8_sha256: None,
        license: GutenbergSourceRecord::public_domain_in_usa_license(),
        fetch_namespace_kind: None,
        fetch_namespace_id: None,
        compression_kind: None,
        archive_member_path: None,
        pre_strip_byte_length: None,
        drop_reason: Some(reason),
        duplicate_of_book_id: None,
        post_strip_byte_length: None,
        post_strip_sha256: None,
        post_charset_body_sha256: None,
        post_charset_token_length: None,
        unmappable_count: None,
        unmappable_density: None,
        split: None,
    }
}

fn canonical_manifest_fixture() -> GutenbergManifest {
    GutenbergManifest {
        schema: GutenbergManifest::schema_id(),
        source_name: GutenbergManifest::source_name_literal(),
        catalog_snapshot_url: "https://www.gutenberg.org/cache/epub/feeds/rdf-files.tar.bz2"
            .to_owned(),
        catalog_snapshot_sha256: hash(1),
        catalog_snapshot_observed_at_utc: "2026-05-19T00:00:00Z".to_owned(),
        catalog_snapshot_last_modified_utc: Some("2026-05-18T12:00:00Z".to_owned()),
        selection_filter_canonical_json:
            "{\"languages_canonical\":[\"en\"],\"rights\":\"public_domain_in_usa\"}".to_owned(),
        selection_filter_sha256: hash(2),
        book_ids: vec![11, 22, 33],
        sources: vec![
            retained_source(11, GutenbergSplit::Train),
            retained_source(22, GutenbergSplit::Val),
            dropped_source(33, GutenbergDropReason::NoSupportedPlaintextFormat),
        ],
        header_regex_pattern: "(?s)^.*?\\*\\*\\* START OF.*?\\*\\*\\*".to_owned(),
        footer_regex_pattern: "(?s)\\*\\*\\* END OF.*$".to_owned(),
        normalization_spec_self_hash: hash(3),
        dedup_policy: GutenbergDedupPolicy::exact_post_strip_charset_body_sha(),
        split_seed_u128: "0123456789abcdef0123456789abcdef".to_owned(),
        split_train_fraction: 0.90,
        split_val_fraction: 0.10,
        train_path: "experiments/S4/corpus/gutenberg-train.bin".to_owned(),
        val_path: "experiments/S4/corpus/gutenberg-val.bin".to_owned(),
        train_sha256: hash(4),
        val_sha256: hash(5),
        train_byte_length: 191,
        val_byte_length: 202,
        train_book_count: 1,
        val_book_count: 1,
        drop_count_total: 1,
        drop_count_no_supported_plaintext_format: 1,
        drop_count_no_plaintext_archive_member: 0,
        drop_count_source_decode_failed: 0,
        drop_count_ambiguous_plaintext_archive: 0,
        drop_count_invalid_utf8: 0,
        drop_count_empty_after_strip: 0,
        drop_count_marker_missing: 0,
        drop_count_unmappable_density: 0,
        drop_count_dedup_collision: 0,
        unmappable_rate_corpus: 0.0,
        raw_byte_policy: GutenbergManifest::raw_byte_policy_literal(),
        retained_book_count_min: 2,
        manifest_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("canonical fixture validates")
}

#[test]
fn gutenberg_manifest_round_trips_and_self_hashes() {
    let manifest = canonical_manifest_fixture();
    let canonical = CanonicalGutenbergManifestWrite::to_vec(&manifest).expect("canonical write");
    let decoded: GutenbergManifest =
        serde_json::from_slice(&canonical).expect("canonical manifest decodes");

    assert_eq!(decoded, manifest);
    assert_eq!(
        decoded
            .compute_self_hash()
            .expect("manifest self hash computes"),
        decoded.manifest_self_hash
    );
    assert_eq!(
        CanonicalGutenbergManifestWrite::to_vec(&decoded).expect("canonical rewrite"),
        canonical
    );
}

#[test]
fn gutenberg_manifest_canonical_write_is_deterministic_across_replays() {
    let baseline = canonical_manifest_fixture();
    let baseline_bytes =
        CanonicalGutenbergManifestWrite::to_vec(&baseline).expect("canonical write");

    for _ in 0..10 {
        let replay = canonical_manifest_fixture();
        assert_eq!(replay.manifest_self_hash, baseline.manifest_self_hash);
        assert_eq!(
            replay
                .compute_self_hash()
                .expect("manifest self hash computes"),
            baseline.manifest_self_hash
        );
        assert_eq!(
            CanonicalGutenbergManifestWrite::to_vec(&replay).expect("canonical replay write"),
            baseline_bytes
        );
    }
}

#[test]
fn gutenberg_manifest_public_json_shape_is_pinned() {
    let manifest = canonical_manifest_fixture();

    assert_eq!(
        serde_json::to_value(&manifest).expect("manifest serializes"),
        serde_json::json!({
            "schema": "gutenberg_manifest.v1",
            "source_name": "Project Gutenberg",
            "catalog_snapshot_url": "https://www.gutenberg.org/cache/epub/feeds/rdf-files.tar.bz2",
            "catalog_snapshot_sha256": hash_json(1),
            "catalog_snapshot_observed_at_utc": "2026-05-19T00:00:00Z",
            "catalog_snapshot_last_modified_utc": "2026-05-18T12:00:00Z",
            "selection_filter_canonical_json": "{\"languages_canonical\":[\"en\"],\"rights\":\"public_domain_in_usa\"}",
            "selection_filter_sha256": hash_json(2),
            "book_ids": [11, 22, 33],
            "sources": [
                {
                    "book_id": 11,
                    "title": "Book 11",
                    "author": "Anonymous",
                    "source_landing_url": "https://www.gutenberg.org/ebooks/11",
                    "mirror_fetch_url": "file://mirror/11.txt",
                    "mirror_snapshot_id": "fixture-2026-05-19",
                    "selected_format": "text/plain;charset=utf-8",
                    "source_blob_sha256": hash_json(0x1b),
                    "pre_strip_utf8_sha256": hash_json(0x2b),
                    "license": "public_domain_in_usa",
                    "fetch_namespace_kind": "content_addressed_cache",
                    "fetch_namespace_id": "fixtures/corpora/gutenberg.toml",
                    "compression_kind": "none",
                    "archive_member_path": null,
                    "pre_strip_byte_length": 139,
                    "drop_reason": null,
                    "duplicate_of_book_id": null,
                    "post_strip_byte_length": 107,
                    "post_strip_sha256": hash_json(0x3b),
                    "post_charset_body_sha256": hash_json(0x4b),
                    "post_charset_token_length": 91,
                    "unmappable_count": 0,
                    "unmappable_density": 0.0,
                    "split": "train"
                },
                {
                    "book_id": 22,
                    "title": "Book 22",
                    "author": "Anonymous",
                    "source_landing_url": "https://www.gutenberg.org/ebooks/22",
                    "mirror_fetch_url": "file://mirror/22.txt",
                    "mirror_snapshot_id": "fixture-2026-05-19",
                    "selected_format": "text/plain;charset=utf-8",
                    "source_blob_sha256": hash_json(0x26),
                    "pre_strip_utf8_sha256": hash_json(0x36),
                    "license": "public_domain_in_usa",
                    "fetch_namespace_kind": "content_addressed_cache",
                    "fetch_namespace_id": "fixtures/corpora/gutenberg.toml",
                    "compression_kind": "none",
                    "archive_member_path": null,
                    "pre_strip_byte_length": 150,
                    "drop_reason": null,
                    "duplicate_of_book_id": null,
                    "post_strip_byte_length": 118,
                    "post_strip_sha256": hash_json(0x46),
                    "post_charset_body_sha256": hash_json(0x56),
                    "post_charset_token_length": 102,
                    "unmappable_count": 0,
                    "unmappable_density": 0.0,
                    "split": "val"
                },
                {
                    "book_id": 33,
                    "title": "Dropped 33",
                    "author": "Anonymous",
                    "source_landing_url": "https://www.gutenberg.org/ebooks/33",
                    "mirror_fetch_url": null,
                    "mirror_snapshot_id": null,
                    "selected_format": null,
                    "source_blob_sha256": null,
                    "pre_strip_utf8_sha256": null,
                    "license": "public_domain_in_usa",
                    "fetch_namespace_kind": null,
                    "fetch_namespace_id": null,
                    "compression_kind": null,
                    "archive_member_path": null,
                    "pre_strip_byte_length": null,
                    "drop_reason": "no_supported_plaintext_format",
                    "duplicate_of_book_id": null,
                    "post_strip_byte_length": null,
                    "post_strip_sha256": null,
                    "post_charset_body_sha256": null,
                    "post_charset_token_length": null,
                    "unmappable_count": null,
                    "unmappable_density": null,
                    "split": null
                }
            ],
            "header_regex_pattern": "(?s)^.*?\\*\\*\\* START OF.*?\\*\\*\\*",
            "footer_regex_pattern": "(?s)\\*\\*\\* END OF.*$",
            "normalization_spec_self_hash": hash_json(3),
            "dedup_policy": {
                "kind": "exact_post_strip_charset_body_sha",
                "notes": "Two retained books with identical post_charset_body_sha256 (i.e. identical body token-id streams excluding <bos>/<eos>) are treated as duplicates; only the lowest book_id is retained. Raw source_blob_sha256 is reported but is not the dedup key, because Gutenberg boilerplate divergence (release notes, edition metadata) can mask body-identical duplicates."
            },
            "split_seed_u128": "0123456789abcdef0123456789abcdef",
            "split_train_fraction": 0.9,
            "split_val_fraction": 0.1,
            "train_path": "experiments/S4/corpus/gutenberg-train.bin",
            "val_path": "experiments/S4/corpus/gutenberg-val.bin",
            "train_sha256": hash_json(4),
            "val_sha256": hash_json(5),
            "train_byte_length": 191,
            "val_byte_length": 202,
            "train_book_count": 1,
            "val_book_count": 1,
            "drop_count_total": 1,
            "drop_count_no_supported_plaintext_format": 1,
            "drop_count_no_plaintext_archive_member": 0,
            "drop_count_source_decode_failed": 0,
            "drop_count_ambiguous_plaintext_archive": 0,
            "drop_count_invalid_utf8": 0,
            "drop_count_empty_after_strip": 0,
            "drop_count_marker_missing": 0,
            "drop_count_unmappable_density": 0,
            "drop_count_dedup_collision": 0,
            "unmappable_rate_corpus": 0.0,
            "raw_byte_policy": "post-strip, post-charset_v1 token-id stream, one octet per token id; <bos>/<eos> inserted at book boundaries (id 80 / 81); <unk> id 82.",
            "retained_book_count_min": 2,
            "manifest_self_hash": manifest.manifest_self_hash
        })
    );
}

#[test]
fn gutenberg_manifest_minimal_fixture_is_canonical_json() {
    let fixture = include_str!("../../fixtures/schemas/s4/gutenberg_manifest_v1_minimal.json");
    let decoded: GutenbergManifest =
        serde_json::from_str(fixture).expect("minimal fixture decodes");
    let canonical = CanonicalGutenbergManifestWrite::to_vec(&decoded).expect("canonical write");

    assert_eq!(decoded.book_ids.len(), 5);
    assert_eq!(decoded.sources.len(), 5);
    assert_eq!(decoded.train_book_count, 3);
    assert_eq!(decoded.val_book_count, 1);
    assert_eq!(decoded.drop_count_total, 1);
    assert_eq!(
        std::str::from_utf8(&canonical).expect("canonical bytes are UTF-8"),
        fixture.trim_end()
    );
}

#[test]
fn gutenberg_manifest_rejects_unordered_book_ids() {
    let mut manifest = canonical_manifest_fixture();
    manifest.book_ids.swap(0, 1);
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("mutated self hash computes");

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::BookIdsNotSorted)
    ));
}

#[test]
fn gutenberg_manifest_rejects_source_order_mismatch() {
    let mut manifest = canonical_manifest_fixture();
    manifest.sources.swap(0, 1);
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("mutated self hash computes");

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::SourceBookIdMismatch { .. })
    ));
}

#[test]
fn gutenberg_manifest_rejects_drop_count_mismatch() {
    let mut manifest = canonical_manifest_fixture();
    manifest.drop_count_no_supported_plaintext_format = 0;
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("mutated self hash computes");

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::DropCountMismatch {
            field: "drop_count_no_supported_plaintext_format",
            ..
        })
    ));
}

#[test]
fn gutenberg_manifest_rejects_retained_source_without_split() {
    let mut manifest = canonical_manifest_fixture();
    manifest.sources[0].split = None;
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("mutated self hash computes");

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::RetainedSourceMissingSplit { book_id: 11 })
    ));
}

#[test]
fn gutenberg_manifest_rejects_retained_count_below_floor() {
    let mut manifest = canonical_manifest_fixture();
    manifest.retained_book_count_min = 3;

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::RetainedBookCountBelowFloor {
            retained_book_count: 2,
            retained_book_count_min: 3
        })
    ));
}

#[test]
fn gutenberg_manifest_rejects_short_split_byte_lengths() {
    let mut manifest = canonical_manifest_fixture();
    manifest.val_byte_length = 127;

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::InvalidSplitByteLength {
            field: "val_byte_length",
            min: 128,
            observed: 127
        })
    ));
}

#[test]
fn gutenberg_manifest_rejects_g_ok_12_drop_field_shape_mismatch() {
    let mut manifest = canonical_manifest_fixture();
    manifest.sources[2].selected_format = Some("text/plain;charset=utf-8".to_owned());

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::DropReasonFieldMustBeNull {
            book_id: 33,
            reason: GutenbergDropReason::NoSupportedPlaintextFormat,
            field: "selected_format"
        })
    ));

    let mut manifest = canonical_manifest_fixture();
    manifest.sources[2].drop_reason = Some(GutenbergDropReason::NoPlaintextArchiveMember);
    manifest.sources[2].source_blob_sha256 = Some(hash(0x66));
    manifest.sources[2].compression_kind = Some(GutenbergCompressionKind::None);
    manifest.drop_count_no_supported_plaintext_format = 0;
    manifest.drop_count_no_plaintext_archive_member = 1;

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::DropReasonCompressionKindMismatch {
            book_id: 33,
            reason: GutenbergDropReason::NoPlaintextArchiveMember,
            expected: GutenbergCompressionKind::Zip,
            observed: Some(GutenbergCompressionKind::None)
        })
    ));
}

#[test]
fn gutenberg_manifest_rejects_dedup_drop_without_duplicate_pointer() {
    let mut manifest = canonical_manifest_fixture();
    manifest.sources[2].drop_reason = Some(GutenbergDropReason::DedupCollision);
    manifest.sources[2].selected_format = Some("text/plain;charset=utf-8".to_owned());
    manifest.sources[2].source_blob_sha256 = Some(hash(0x66));
    manifest.sources[2].pre_strip_utf8_sha256 = Some(hash(0x67));
    manifest.sources[2].pre_strip_byte_length = Some(400);
    manifest.drop_count_no_supported_plaintext_format = 0;
    manifest.drop_count_dedup_collision = 1;
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("mutated self hash computes");

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::DedupDropMissingDuplicate { book_id: 33 })
    ));
}

#[test]
fn gutenberg_manifest_rejects_self_hash_mismatch() {
    let mut manifest = canonical_manifest_fixture();
    manifest.train_byte_length += 1;

    assert!(matches!(
        CanonicalGutenbergManifestWrite::to_vec(&manifest),
        Err(GutenbergManifestError::SelfHashMismatch { .. })
    ));
}

#[test]
fn gutenberg_manifest_deserialization_rejects_unknown_and_bad_literals() {
    let mut value = serde_json::to_value(canonical_manifest_fixture()).expect("manifest json");
    value["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<GutenbergManifest>(value).is_err());

    let mut value = serde_json::to_value(canonical_manifest_fixture()).expect("manifest json");
    value["schema"] = serde_json::json!("gutenberg_manifest.v2");
    assert!(serde_json::from_value::<GutenbergManifest>(value).is_err());

    let mut value = serde_json::to_value(canonical_manifest_fixture()).expect("manifest json");
    value["sources"][0]["license"] = serde_json::json!("public_domain_worldwide");
    assert!(serde_json::from_value::<GutenbergManifest>(value).is_err());
}

#[test]
fn gutenberg_manifest_json_rejects_negative_drop_counts() {
    for field in [
        "drop_count_total",
        "drop_count_no_supported_plaintext_format",
        "drop_count_no_plaintext_archive_member",
        "drop_count_source_decode_failed",
        "drop_count_ambiguous_plaintext_archive",
        "drop_count_invalid_utf8",
        "drop_count_empty_after_strip",
        "drop_count_marker_missing",
        "drop_count_unmappable_density",
        "drop_count_dedup_collision",
    ] {
        let mut value = serde_json::to_value(canonical_manifest_fixture()).expect("manifest json");
        value[field] = serde_json::json!(-1);

        assert!(
            serde_json::from_value::<GutenbergManifest>(value).is_err(),
            "{field} should reject negative JSON numbers"
        );
    }
}
