use std::path::PathBuf;

use gbf_data::{
    CorpusManifestError, SplitRole, TinyStoriesManifest, load_train_bytes, load_val_bytes,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-data has workspace parent")
        .to_path_buf()
}

#[test]
fn tinystories_manifest_parses_and_pins_source_metadata() {
    let manifest = TinyStoriesManifest::from_toml_file(
        workspace_root().join("fixtures/corpora/tinystories.toml"),
    )
    .expect("TinyStories manifest parses");

    assert_eq!(manifest.schema, "tinystories_manifest.v1");
    assert_eq!(manifest.schema_version, "1.0.0");
    assert_eq!(manifest.corpus_id, "tinystories");
    assert_eq!(manifest.source_name, manifest.source.name);
    assert_eq!(manifest.source_url, manifest.source.url);
    assert_eq!(manifest.source.name, "roneneldan/TinyStories");
    assert_eq!(
        manifest.source.url,
        "https://huggingface.co/datasets/roneneldan/TinyStories"
    );
    assert_eq!(manifest.source.license, "CDLA-Sharing-1.0");
    assert_eq!(manifest.source.decompression, "none");
    assert_eq!(manifest.raw_root, "../../corpus/tinystories/raw");
    assert!(
        manifest
            .raw_byte_policy
            .contains("post-decompression bytes")
    );
    assert!(manifest.raw_byte_policy.contains("no normalization"));
    assert_eq!(manifest.story_separator.as_bytes(), b"<|endoftext|>");
    assert!(manifest.s1_policy.contains("ordinary input bytes"));
}

#[test]
fn tinystories_manifest_pins_s1_raw_files() {
    let manifest = TinyStoriesManifest::from_toml_file(
        workspace_root().join("fixtures/corpora/tinystories.toml"),
    )
    .expect("TinyStories manifest parses");

    let train = manifest.file(SplitRole::Train);
    assert_eq!(
        manifest.train_path,
        "../../corpus/tinystories/raw/TinyStoriesV2-GPT4-train.txt"
    );
    assert_eq!(
        manifest.split_path(SplitRole::Train),
        workspace_root()
            .join("fixtures/corpora")
            .join("../../corpus/tinystories/raw/TinyStoriesV2-GPT4-train.txt")
    );
    assert_eq!(manifest.train_sha256, train.sha256);
    assert_eq!(train.local_filename, "TinyStoriesV2-GPT4-train.txt");
    assert_eq!(
        train.url,
        "https://huggingface.co/datasets/roneneldan/TinyStories/resolve/main/TinyStoriesV2-GPT4-train.txt"
    );
    assert_eq!(
        train.sha256.to_string(),
        "sha256:6418d412de72888f52b5142c761ac21a582f7d1166f0bfbdb5f03ccfdec90443"
    );
    assert_eq!(train.byte_length, 2_227_753_162);
    assert_eq!(train.story_count, 2_717_699);

    let validation = manifest.file(SplitRole::Validation);
    assert_eq!(
        manifest.val_path,
        "../../corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt"
    );
    assert_eq!(
        manifest.split_path(SplitRole::Validation),
        workspace_root()
            .join("fixtures/corpora")
            .join("../../corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt")
    );
    assert_eq!(manifest.val_sha256, validation.sha256);
    assert_eq!(validation.local_filename, "TinyStoriesV2-GPT4-valid.txt");
    assert_eq!(
        validation.url,
        "https://huggingface.co/datasets/roneneldan/TinyStories/resolve/main/TinyStoriesV2-GPT4-valid.txt"
    );
    assert_eq!(
        validation.sha256.to_string(),
        "sha256:6874bae9a4c1a4e7edcf0e53b86c17817e9cf881fc75ff2368da457b80c0585d"
    );
    assert_eq!(
        manifest
            .val_shuffle_deadeef_sha256
            .expect("TinyStories shuffle pin")
            .to_string(),
        "sha256:33ab115b5d230b6286fd39347e7e542bb7663ed148d80e16fc3de1a866f60388"
    );
    assert_eq!(
        manifest
            .val_shuffle_deadeef_pinned_at_pass_version
            .as_deref(),
        Some("F-S1.09.bd-2who.2026-05-09")
    );
    assert_eq!(validation.byte_length, 22_502_601);
    assert_eq!(validation.story_count, 27_630);
}

#[test]
fn manifest_file_relative_paths_resolve_from_manifest_directory_not_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_dir = dir.path().join("manifests");
    std::fs::create_dir(&manifest_dir).expect("manifest dir");
    let train = b"train relative bytes";
    let val = b"val relative bytes";
    std::fs::write(manifest_dir.join("train.bytes"), train).expect("write train");
    std::fs::write(manifest_dir.join("val.bytes"), val).expect("write val");
    let manifest_path = manifest_dir.join("manifest.toml");
    std::fs::write(
        &manifest_path,
        format!(
            r#"
schema = "tinystories_manifest.v1"
schema_version = "1.0.0"
corpus_id = "relative-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "train.bytes"
val_path = "val.bytes"
train_sha256 = "{}"
val_sha256 = "{}"
raw_root = "."
raw_byte_policy = "post-decompression bytes; no normalization"
story_separator = "<|endoftext|>"
s1_policy = "test"
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
url = "file://train.bytes"
local_filename = "train.bytes"
sha256 = "{}"
byte_length = {}
story_count = 1

[splits.validation]
role = "validation"
url = "file://val.bytes"
local_filename = "val.bytes"
sha256 = "{}"
byte_length = {}
story_count = 1
"#,
            gbf_foundation::sha256(train),
            gbf_foundation::sha256(val),
            gbf_foundation::sha256(train),
            train.len(),
            gbf_foundation::sha256(val),
            val.len()
        ),
    )
    .expect("write manifest");

    let manifest = TinyStoriesManifest::from_toml_file(&manifest_path).expect("manifest parses");

    assert_ne!(
        std::env::current_dir().expect("cwd"),
        manifest_dir,
        "test must prove paths do not depend on caller cwd"
    );
    assert_eq!(
        manifest.split_path(SplitRole::Train),
        manifest_dir.join("train.bytes")
    );
    assert_eq!(load_train_bytes(&manifest).expect("train loads"), train);
    assert_eq!(load_val_bytes(&manifest).expect("val loads"), val);
}

#[test]
fn manifest_file_verification_checks_tiny_fixture_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("tiny.txt");
    let bytes = b"Once there was a test.<|endoftext|>";
    std::fs::write(&path, bytes).expect("write tiny fixture");

    let manifest = TinyStoriesManifest::from_toml_str(&format!(
        r#"
schema = "tinystories_manifest.v1"
schema_version = "1.0.0"
corpus_id = "tiny-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "{root}/tiny.txt"
val_path = "{root}/tiny.txt"
train_sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
val_sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
raw_root = "{root}"
raw_byte_policy = "post-decompression bytes; no normalization"
story_separator = "<|endoftext|>"
s1_policy = "test"
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
local_filename = "tiny.txt"
sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
byte_length = 35
story_count = 1

[splits.validation]
role = "validation"
url = "https://example.invalid/valid"
local_filename = "tiny.txt"
sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
byte_length = 35
story_count = 1
"#,
        root = dir.path().display()
    ))
    .expect("fixture manifest parses");

    manifest
        .file(SplitRole::Train)
        .verify_bytes(bytes)
        .expect("bytes verify");
    manifest
        .file(SplitRole::Validation)
        .verify_file(path)
        .expect("file verifies");
}

#[test]
fn manifest_loaders_return_exact_raw_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let train_path = dir.path().join("train.bytes");
    let val_path = dir.path().join("val.bytes");
    let train = b"alpha\n<|endoftext|>\r\nbeta\x00\xff";
    let val = b"validation keeps every byte\n<|endoftext|>";
    std::fs::write(&train_path, train).expect("write train fixture");
    std::fs::write(&val_path, val).expect("write val fixture");

    let manifest = TinyStoriesManifest::from_toml_str(&format!(
        r#"
schema = "tinystories_manifest.v1"
schema_version = "1.0.0"
corpus_id = "tiny-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "{train_path}"
val_path = "{val_path}"
train_sha256 = "sha256:09d92f5489e3c3ab12305633fcecd2c91da75ec7a05080b4622099fbca33866b"
val_sha256 = "sha256:34a8ec4ed83cad0447c17c4a762a395131a3928f3fb4a5182e8fc44ee92a8583"
raw_root = "{root}"
raw_byte_policy = "post-decompression bytes; no normalization"
story_separator = "<|endoftext|>"
s1_policy = "test"
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
sha256 = "sha256:09d92f5489e3c3ab12305633fcecd2c91da75ec7a05080b4622099fbca33866b"
byte_length = 27
story_count = 2

[splits.validation]
role = "validation"
url = "https://example.invalid/valid"
local_filename = "{val_path}"
sha256 = "sha256:34a8ec4ed83cad0447c17c4a762a395131a3928f3fb4a5182e8fc44ee92a8583"
byte_length = 41
story_count = 1
"#,
        root = dir.path().display(),
        train_path = train_path.display(),
        val_path = val_path.display()
    ))
    .expect("fixture manifest parses");

    assert_eq!(load_train_bytes(&manifest).expect("train loads"), train);
    assert_eq!(load_val_bytes(&manifest).expect("val loads"), val);
}

#[test]
fn manifest_file_verification_rejects_wrong_length_and_sha() {
    let manifest = TinyStoriesManifest::from_toml_str(
        r#"
schema = "tinystories_manifest.v1"
schema_version = "1.0.0"
corpus_id = "tiny-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "tiny.txt"
val_path = "tiny.txt"
train_sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
val_sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
raw_root = "corpus/tiny/raw"
raw_byte_policy = "post-decompression bytes; no normalization"
story_separator = "<|endoftext|>"
s1_policy = "test"
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
local_filename = "tiny.txt"
sha256 = "sha256:6b985f1e6ae7a5789013d40da3f482cec0fff84886ba23c08e6c41e9d6803dc8"
byte_length = 34
story_count = 1

[splits.validation]
role = "validation"
url = "https://example.invalid/valid"
local_filename = "tiny.txt"
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
byte_length = 35
story_count = 1
"#,
    )
    .expect("fixture manifest parses");

    let bytes = b"Once there was a test.<|endoftext|>";
    assert!(matches!(
        manifest.file(SplitRole::Train).verify_bytes(bytes),
        Err(CorpusManifestError::ByteLengthMismatch { .. })
    ));
    assert!(matches!(
        manifest.file(SplitRole::Validation).verify_bytes(bytes),
        Err(CorpusManifestError::Sha256Mismatch { .. })
    ));
}

#[test]
fn manifest_rejects_wrong_schema() {
    let error = TinyStoriesManifest::from_toml_str(
        r#"
schema = "tinystories_manifest.v2"
schema_version = "1.0.0"
corpus_id = "tiny-fixture"
dataset_version = "test"
source_name = "fixture"
source_url = "https://example.invalid/fixture"
train_path = "train.bytes"
val_path = "val.bytes"
train_sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
val_sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
raw_root = "corpus/tiny/raw"
raw_byte_policy = "post-decompression bytes; no normalization"
story_separator = "<|endoftext|>"
s1_policy = "test"
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
local_filename = "train.bytes"
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
byte_length = 0
story_count = 0

[splits.validation]
role = "validation"
url = "https://example.invalid/valid"
local_filename = "val.bytes"
sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
byte_length = 0
story_count = 0
"#,
    )
    .expect_err("wrong schema rejects");

    assert!(matches!(
        error,
        CorpusManifestError::SchemaMismatch {
            expected: "tinystories_manifest.v1",
            observed,
        } if observed == "tinystories_manifest.v2"
    ));
}
