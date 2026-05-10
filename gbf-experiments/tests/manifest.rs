use std::path::{Path, PathBuf};

use gbf_data::CorpusManifestError;
use gbf_experiments::s1::device_profile::{S1CpuDeterministic, enforce_with_environment};
use gbf_experiments::s1::manifest::{
    S1ManifestError, load_train_bytes, load_val_bytes, read_tinystories_manifest, verified_corpus,
    verified_corpus_after_enforcement,
};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn manifest_text(train_path: &Path, val_path: &Path, train: &[u8], val: &[u8]) -> String {
    manifest_text_with_sha(
        train_path,
        val_path,
        &sha256_hex(train),
        &sha256_hex(val),
        train.len(),
        val.len(),
        "tinystories_manifest.v1",
    )
}

fn manifest_text_with_sha(
    train_path: &Path,
    val_path: &Path,
    train_sha: &str,
    val_sha: &str,
    train_len: usize,
    val_len: usize,
    schema: &str,
) -> String {
    format!(
        r#"
schema = "{schema}"
schema_version = "1.0.0"
corpus_id = "s1-tiny-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "{train_path}"
val_path = "{val_path}"
train_sha256 = "{train_sha}"
val_sha256 = "{val_sha}"
raw_root = "{root}"
raw_byte_policy = "post-decompression bytes; no normalization; no truncation"
story_separator = "<|endoftext|>"
s1_policy = "<|endoftext|> remains ordinary input bytes"
deferred_scope = []

[source]
name = "fixture"
url = "https://example.invalid/fixture"
dataset_card_url = "https://example.invalid/card"
license = "test"
license_url = "https://example.invalid/license"
downloaded_at = "2026-05-09"
decompression = "none"

[splits.train]
role = "train"
url = "https://example.invalid/train"
local_filename = "{train_path}"
sha256 = "{train_sha}"
byte_length = {train_len}
story_count = 2

[splits.validation]
role = "validation"
url = "https://example.invalid/valid"
local_filename = "{val_path}"
sha256 = "{val_sha}"
byte_length = {val_len}
story_count = 1
"#,
        root = train_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .display(),
        train_path = train_path.display(),
        val_path = val_path.display(),
    )
}

fn write_fixture(dir: &Path, train: &[u8], val: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
    let train_path = dir.join("train.bytes");
    let val_path = dir.join("val.bytes");
    let manifest_path = dir.join("tinystories.toml");
    std::fs::write(&train_path, train).expect("write train fixture");
    std::fs::write(&val_path, val).expect("write val fixture");
    std::fs::write(
        &manifest_path,
        manifest_text(&train_path, &val_path, train, val),
    )
    .expect("write manifest fixture");
    (manifest_path, train_path, val_path)
}

fn device_enforcement() -> gbf_experiments::s1::device_profile::DeviceProfileEnforcement {
    enforce_with_environment(
        &S1CpuDeterministic::canonical(),
        [
            ("BURN_DETERMINISTIC", "1"),
            ("BURN_NDARRAY_NUM_THREADS", "1"),
            ("OMP_NUM_THREADS", "1"),
            ("RAYON_NUM_THREADS", "1"),
        ],
    )
    .expect("fixture environment enforces")
}

#[test]
fn verified_corpus_preserves_literal_separator_and_validation_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train = b"first story\n<|endoftext|>\r\nsecond story\x00";
    let val = b"validation bytes\n<|endoftext|>\nnot truncated";
    let (manifest_path, _, _) = write_fixture(dir.path(), train, val);
    let manifest = read_tinystories_manifest(&manifest_path).expect("manifest reads");

    let corpus = verified_corpus_after_enforcement(&manifest, device_enforcement())
        .expect("corpus verifies");

    assert_eq!(corpus.train, train);
    assert_eq!(corpus.val, val);
    assert!(
        corpus
            .train
            .windows(b"<|endoftext|>".len())
            .any(|w| w == b"<|endoftext|>")
    );
}

#[test]
fn train_sha_mismatch_returns_typed_error_with_file_diagnostic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train = b"original train bytes";
    let val = b"valid val bytes";
    let train_path = dir.path().join("train.bytes");
    let val_path = dir.path().join("val.bytes");
    std::fs::write(&train_path, b"tampered train bytes").expect("write train");
    std::fs::write(&val_path, val).expect("write val");
    let manifest = gbf_experiments::s1::manifest::TinyStoriesManifest::from_toml_str(
        &manifest_text(&train_path, &val_path, train, val),
    )
    .expect("manifest parses");

    let error = load_train_bytes(&manifest).expect_err("sha mismatch rejects");

    assert!(matches!(error, CorpusManifestError::Sha256Mismatch { .. }));
    assert!(
        error
            .to_string()
            .contains(&train_path.display().to_string())
    );
}

#[test]
fn missing_file_returns_io_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train_path = dir.path().join("missing-train.bytes");
    let val_path = dir.path().join("val.bytes");
    let val = b"valid val bytes";
    std::fs::write(&val_path, val).expect("write val");
    let manifest = gbf_experiments::s1::manifest::TinyStoriesManifest::from_toml_str(
        &manifest_text(&train_path, &val_path, b"missing train bytes", val),
    )
    .expect("manifest parses");

    let error = load_train_bytes(&manifest).expect_err("missing train rejects");

    assert!(matches!(error, CorpusManifestError::Io { .. }));
    assert!(error.to_string().contains("missing-train.bytes"));
}

#[test]
fn wrong_schema_returns_manifest_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train_path = dir.path().join("train.bytes");
    let val_path = dir.path().join("val.bytes");
    let manifest_path = dir.path().join("tinystories.toml");
    std::fs::write(
        &manifest_path,
        manifest_text_with_sha(
            &train_path,
            &val_path,
            &sha256_hex(b""),
            &sha256_hex(b""),
            0,
            0,
            "tinystories_manifest.v2",
        ),
    )
    .expect("write manifest");

    let error = read_tinystories_manifest(&manifest_path).expect_err("wrong schema rejects");

    assert!(matches!(
        error,
        CorpusManifestError::SchemaMismatch {
            expected: "tinystories_manifest.v1",
            observed,
        } if observed == "tinystories_manifest.v2"
    ));
}

#[test]
fn val_loader_returns_entire_validation_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train = b"train bytes";
    let val = b"prefix\n<|endoftext|>\nmid\x00suffix\n";
    let (manifest_path, _, _) = write_fixture(dir.path(), train, val);
    let manifest = read_tinystories_manifest(&manifest_path).expect("manifest reads");

    assert_eq!(load_val_bytes(&manifest).expect("val loads"), val);
}

#[test]
fn verified_corpus_enforces_device_profile_before_loading_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train_path = dir.path().join("missing-train.bytes");
    let val_path = dir.path().join("missing-val.bytes");
    let manifest = gbf_experiments::s1::manifest::TinyStoriesManifest::from_toml_str(
        &manifest_text(&train_path, &val_path, b"train", b"val"),
    )
    .expect("manifest parses");

    let error = verified_corpus(&manifest).expect_err("process environment rejects first");

    assert!(matches!(error, S1ManifestError::DeviceProfile(_)));
}

proptest::proptest! {
    #[test]
    fn train_loader_echoes_random_bytes(raw in proptest::collection::vec(any::<u8>(), 0..512)) {
        let dir = tempfile::tempdir().expect("tempdir");
        let val = b"val";
        let (manifest_path, _, _) = write_fixture(dir.path(), &raw, val);
        let manifest = read_tinystories_manifest(&manifest_path).expect("manifest reads");

        proptest::prop_assert_eq!(load_train_bytes(&manifest).expect("train loads"), raw);
    }
}
