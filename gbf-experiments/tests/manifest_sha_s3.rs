#![cfg(feature = "s3")]

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use gbf_artifact::LexicalSpec_v1;
use gbf_data::charset_v1::{CharsetInputs, normalize_raw, s3_charset_v1};
use gbf_data::{
    SplitRole, TinyStoriesV2Manifest, read_tinystories_manifest, read_tinystories_v2_manifest,
};
use gbf_foundation::{Hash256, sha256};
use serde::Serialize;

#[test]
fn tinystories_v2_manifest_sha_fields_match_source_bytes() {
    let manifest_path = tinystories_v2_manifest_path();
    let manifest =
        read_tinystories_v2_manifest(&manifest_path).expect("TinyStories.v2 manifest reads");
    let evidence = ManifestEvidence::compute(&manifest).expect("manifest evidence computes");

    eprintln!("raw_train_sha256={}", evidence.raw_train_sha256);
    eprintln!("raw_val_sha256={}", evidence.raw_val_sha256);
    eprintln!(
        "fixture_raw_train_sha256={}",
        evidence.fixture_raw_train_sha256
    );
    eprintln!("fixture_raw_val_sha256={}", evidence.fixture_raw_val_sha256);
    eprintln!("train_post_sha256={}", evidence.train_post_sha256);
    eprintln!("val_post_sha256={}", evidence.val_post_sha256);
    eprintln!("charset_v1_sha256={}", evidence.charset_v1_sha256);
    eprintln!("chapter_sha256={}", evidence.chapter_sha256);
    eprintln!("raw_train_byte_count={}", evidence.raw_train_byte_count);
    eprintln!("raw_val_byte_count={}", evidence.raw_val_byte_count);
    eprintln!(
        "fixture_raw_train_byte_count={}",
        evidence.fixture_raw_train_byte_count
    );
    eprintln!(
        "fixture_raw_val_byte_count={}",
        evidence.fixture_raw_val_byte_count
    );
    eprintln!("train_post_char_count={}", evidence.train_post_char_count);
    eprintln!("val_post_char_count={}", evidence.val_post_char_count);
    eprintln!("chapter_char_count={}", evidence.chapter_char_count);

    maybe_write_verify_ndjson(&manifest, &evidence).expect("optional NDJSON verification writes");

    assert_hash_field(
        "raw_train_sha256",
        manifest.raw_train_sha256,
        evidence.raw_train_sha256,
    );
    assert_hash_field(
        "raw_val_sha256",
        manifest.raw_val_sha256,
        evidence.raw_val_sha256,
    );
    assert_hash_field(
        "fixture_raw_train_sha256",
        manifest.fixture_raw_train_sha256,
        evidence.fixture_raw_train_sha256,
    );
    assert_hash_field(
        "fixture_raw_val_sha256",
        manifest.fixture_raw_val_sha256,
        evidence.fixture_raw_val_sha256,
    );
    assert_hash_field(
        "train_post_sha256",
        manifest.train_post_sha256,
        evidence.train_post_sha256,
    );
    assert_hash_field(
        "val_post_sha256",
        manifest.val_post_sha256,
        evidence.val_post_sha256,
    );
    assert_hash_field(
        "charset_v1_sha256",
        manifest.charset_v1_sha256,
        evidence.charset_v1_sha256,
    );
    assert_hash_field(
        "chapter_sha256",
        manifest.chapter_sha256,
        evidence.chapter_sha256,
    );
    assert_eq!(
        manifest.raw_train_byte_count, evidence.raw_train_byte_count,
        "raw_train_byte_count"
    );
    assert_eq!(
        manifest.raw_val_byte_count, evidence.raw_val_byte_count,
        "raw_val_byte_count"
    );
    assert_eq!(
        manifest.fixture_raw_train_byte_count, evidence.fixture_raw_train_byte_count,
        "fixture_raw_train_byte_count"
    );
    assert_eq!(
        manifest.fixture_raw_val_byte_count, evidence.fixture_raw_val_byte_count,
        "fixture_raw_val_byte_count"
    );
    assert_eq!(
        manifest.train_post_char_count, evidence.train_post_char_count,
        "train_post_char_count"
    );
    assert_eq!(
        manifest.val_post_char_count, evidence.val_post_char_count,
        "val_post_char_count"
    );
    assert_eq!(
        manifest.chapter_char_count, evidence.chapter_char_count,
        "chapter_char_count"
    );
}

#[derive(Debug)]
struct ManifestEvidence {
    raw_train_sha256: Hash256,
    raw_val_sha256: Hash256,
    fixture_raw_train_sha256: Hash256,
    fixture_raw_val_sha256: Hash256,
    train_post_sha256: Hash256,
    val_post_sha256: Hash256,
    charset_v1_sha256: Hash256,
    chapter_sha256: Hash256,
    raw_train_byte_count: u64,
    raw_val_byte_count: u64,
    fixture_raw_train_byte_count: u64,
    fixture_raw_val_byte_count: u64,
    train_post_char_count: u64,
    val_post_char_count: u64,
    chapter_char_count: u64,
}

impl ManifestEvidence {
    fn compute(
        manifest: &TinyStoriesV2Manifest,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let source_manifest =
            read_tinystories_manifest(manifest.resolve_path(&manifest.source_manifest_path))?;
        let fixture_raw_train =
            std::fs::read(manifest.resolve_path(&manifest.fixture_raw_train_path))?;
        let fixture_raw_val = std::fs::read(manifest.resolve_path(&manifest.fixture_raw_val_path))?;
        let chapter = std::fs::read(manifest.resolve_path(&manifest.held_out_chapter_path))?;

        let product = s3_charset_v1(CharsetInputs {
            raw_train_examples: vec![fixture_raw_train.clone()],
            raw_val_examples: vec![fixture_raw_val.clone()],
            spec: LexicalSpec_v1::pinned(),
        })?;
        let chapter_stats = normalize_raw(&chapter)?;
        assert!(
            !chapter_stats.dropped,
            "held-out chapter fixture must survive charset_v1 normalization"
        );

        Ok(Self {
            raw_train_sha256: source_manifest.file(SplitRole::Train).sha256,
            raw_val_sha256: source_manifest.file(SplitRole::Validation).sha256,
            fixture_raw_train_sha256: sha256(&fixture_raw_train),
            fixture_raw_val_sha256: sha256(&fixture_raw_val),
            train_post_sha256: product.train_post_sha256,
            val_post_sha256: product.val_post_sha256,
            charset_v1_sha256: product.charset_v1_sha256,
            chapter_sha256: sha256(&chapter),
            raw_train_byte_count: source_manifest.file(SplitRole::Train).byte_length,
            raw_val_byte_count: source_manifest.file(SplitRole::Validation).byte_length,
            fixture_raw_train_byte_count: fixture_raw_train.len() as u64,
            fixture_raw_val_byte_count: fixture_raw_val.len() as u64,
            train_post_char_count: product.train_post.len() as u64,
            val_post_char_count: product.val_post.len() as u64,
            chapter_char_count: chapter_stats.tokens.len() as u64,
        })
    }
}

#[derive(Serialize)]
struct VerifyRecord<'a> {
    event: &'static str,
    field: &'a str,
    expected: String,
    observed: String,
}

fn maybe_write_verify_ndjson(
    manifest: &TinyStoriesV2Manifest,
    evidence: &ManifestEvidence,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Ok(path) = std::env::var("S3_MANIFEST_VERIFY_NDJSON") else {
        return Ok(());
    };

    let records = [
        record(
            "raw_train_sha256",
            manifest.raw_train_sha256,
            evidence.raw_train_sha256,
        ),
        record(
            "raw_val_sha256",
            manifest.raw_val_sha256,
            evidence.raw_val_sha256,
        ),
        record(
            "fixture_raw_train_sha256",
            manifest.fixture_raw_train_sha256,
            evidence.fixture_raw_train_sha256,
        ),
        record(
            "fixture_raw_val_sha256",
            manifest.fixture_raw_val_sha256,
            evidence.fixture_raw_val_sha256,
        ),
        record(
            "train_post_sha256",
            manifest.train_post_sha256,
            evidence.train_post_sha256,
        ),
        record(
            "val_post_sha256",
            manifest.val_post_sha256,
            evidence.val_post_sha256,
        ),
        record(
            "charset_v1_sha256",
            manifest.charset_v1_sha256,
            evidence.charset_v1_sha256,
        ),
        record(
            "chapter_sha256",
            manifest.chapter_sha256,
            evidence.chapter_sha256,
        ),
    ];

    let mut file = File::create(path)?;
    for record in records {
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

fn record(field: &str, expected: Hash256, observed: Hash256) -> VerifyRecord<'_> {
    VerifyRecord {
        event: "s3_manifest_verify",
        field,
        expected: expected.to_string(),
        observed: observed.to_string(),
    }
}

fn assert_hash_field(field: &'static str, expected: Hash256, observed: Hash256) {
    assert_eq!(expected, observed, "{field}");
}

fn tinystories_v2_manifest_path() -> PathBuf {
    workspace_root().join("fixtures/corpora/tinystories.v2.toml")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
