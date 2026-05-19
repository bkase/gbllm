#![cfg(feature = "s3")]

use std::path::{Path, PathBuf};

use gbf_data::{TINYSTORIES_V2_MANIFEST_SCHEMA, TinyStoriesV2Manifest};

#[test]
fn tinystories_v2_manifest_round_trips_through_toml_reader() {
    let text = std::fs::read_to_string(tinystories_v2_manifest_path())
        .expect("TinyStories.v2 manifest text reads");
    let manifest = TinyStoriesV2Manifest::from_toml_str(&text).expect("manifest parses");
    let encoded = toml::to_string_pretty(&manifest).expect("manifest serializes");
    let decoded =
        TinyStoriesV2Manifest::from_toml_str(&encoded).expect("serialized manifest parses");

    assert_eq!(decoded, manifest);
    assert_eq!(manifest.schema, TINYSTORIES_V2_MANIFEST_SCHEMA);
    assert!(
        manifest.fixture_mode,
        "CI manifest remains explicit about fixture-vs-real corpus ownership"
    );
    assert_eq!(manifest.post_hash_input, "fixture_raw_bytes");
    assert_eq!(
        manifest.raw_train_sha256.to_string(),
        "sha256:6418d412de72888f52b5142c761ac21a582f7d1166f0bfbdb5f03ccfdec90443"
    );
    assert_eq!(
        manifest.raw_val_sha256.to_string(),
        "sha256:6874bae9a4c1a4e7edcf0e53b86c17817e9cf881fc75ff2368da457b80c0585d"
    );
    assert_eq!(
        manifest.fixture_raw_train_sha256.to_string(),
        "sha256:d13fee288566b858b4102d88ca899a019f41b1125d607d32f94b801218708473"
    );
    assert_eq!(
        manifest.fixture_raw_val_sha256.to_string(),
        "sha256:5a3fa6cfb7b244ef25b7b6647838934df030556bbff19048d12df671fbf1200b"
    );
    assert_eq!(manifest.prompt_offsets.len(), 8);
    assert_eq!(&manifest.prompt_offsets[..3], &[0, 64, 128]);
    assert_eq!(manifest.agreement_prompt_count, 3);
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
