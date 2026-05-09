use std::path::{Path, PathBuf};
use std::process::Command;

use gbf_experiments::s1::manifest::{load_train_bytes, load_val_bytes, read_tinystories_manifest};
use gbf_experiments::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
use gbf_experiments::s1::rng::{InitRng, uniform_u64_inclusive};
use gbf_foundation::{Hash256, sha256};

const FIXTURE_DIR: &str = "gbf-experiments/tests/fixtures/tiny_corpus";
const MASTER_SEED: u64 = 0xC0FFEE;
const VAL_SEED_XOR_MASK: u64 = 0xA5A5_A5A5_A5A5_A5A5;
const TRAIN_LEN: usize = 16_384;
const VAL_LEN: usize = 4_096;
const SEPARATOR: &[u8] = b"<|endoftext|>";
const SEPARATOR_OFFSET: usize = 1_024;
const RNG_PREFIX_LEN: usize = 64;
const TRAIN_SHA: &str = "sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f";
const VAL_SHA: &str = "sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4";
const VAL_SHUFFLE_SHA: &str =
    "sha256:494850f839225dbcc5fabb1496b3655e98ccc9844c31b017b55916a4fb91bed2";

#[test]
fn tiny_fixture_manifest_loads_and_pins_hashes() {
    let manifest = read_tinystories_manifest(manifest_path()).expect("tiny manifest");

    assert_eq!(manifest.corpus_id, "s1-tiny-fixture");
    assert_eq!(manifest.source_name, "synthetic-tiny-fixture");
    assert_eq!(manifest.train_path, "train.bytes");
    assert_eq!(manifest.val_path, "val.bytes");
    assert_eq!(manifest.raw_root, ".");
    assert_eq!(manifest.train_sha256, hash(TRAIN_SHA));
    assert_eq!(manifest.val_sha256, hash(VAL_SHA));
    assert_eq!(
        manifest.val_shuffle_deadeef_sha256,
        Some(hash(VAL_SHUFFLE_SHA))
    );
    assert_eq!(manifest.splits.train.byte_length, TRAIN_LEN as u64);
    assert_eq!(manifest.splits.validation.byte_length, VAL_LEN as u64);
    assert_eq!(manifest.splits.train.story_count, 2);
    assert_eq!(manifest.splits.validation.story_count, 1);

    let train = load_train_bytes(&manifest).expect("train loads");
    let val = load_val_bytes(&manifest).expect("val loads");
    assert_eq!(train.len(), TRAIN_LEN);
    assert_eq!(val.len(), VAL_LEN);
    assert_eq!(sha256(&train), hash(TRAIN_SHA));
    assert_eq!(sha256(&val), hash(VAL_SHA));
}

#[test]
fn checked_in_bytes_match_rust_init_rng_prefixes_and_story_counts() {
    let manifest = read_tinystories_manifest(manifest_path()).expect("tiny manifest");
    let train = std::fs::read(fixture_path("train.bytes")).expect("train bytes");
    let val = std::fs::read(fixture_path("val.bytes")).expect("val bytes");

    assert_eq!(
        &train[..RNG_PREFIX_LEN],
        generated_prefix(MASTER_SEED, RNG_PREFIX_LEN).as_slice()
    );
    assert_eq!(
        &val[..RNG_PREFIX_LEN],
        generated_prefix(MASTER_SEED ^ VAL_SEED_XOR_MASK, RNG_PREFIX_LEN).as_slice()
    );
    assert_eq!(
        manifest.splits.train.story_count,
        story_count_from_separators(&train)
    );
    assert_eq!(
        manifest.splits.validation.story_count,
        story_count_from_separators(&val)
    );
}

#[test]
fn documented_separator_offset_is_literal_and_unique() {
    let train = std::fs::read(fixture_path("train.bytes")).expect("train bytes");

    assert_eq!(
        &train[SEPARATOR_OFFSET..SEPARATOR_OFFSET + SEPARATOR.len()],
        SEPARATOR
    );
    let offsets = train
        .windows(SEPARATOR.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == SEPARATOR).then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(offsets, vec![SEPARATOR_OFFSET]);
}

#[test]
fn validation_shuffle_matches_manifest_pin() {
    let manifest = read_tinystories_manifest(manifest_path()).expect("tiny manifest");
    let val = load_val_bytes(&manifest).expect("val loads");
    let shuffled = fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED);

    assert_eq!(sha256(shuffled), hash(VAL_SHUFFLE_SHA));
    assert_eq!(
        manifest.val_shuffle_deadeef_sha256,
        Some(hash(VAL_SHUFFLE_SHA))
    );
}

#[test]
fn generation_recipe_records_literal_source_of_truth_values() {
    let recipe = std::fs::read_to_string(fixture_path("generation_recipe.md")).expect("recipe");

    assert!(recipe.contains("master_seed: `0xc0ffee`"));
    assert!(recipe.contains("validation seed derivation: `master_seed ^ 0xa5a5a5a5a5a5a5a5`"));
    assert!(recipe.contains("separator location: `train.bytes` byte offset `1024`"));
    assert!(recipe.contains(TRAIN_SHA));
    assert!(recipe.contains(VAL_SHA));
    assert!(recipe.contains(VAL_SHUFFLE_SHA));
}

#[test]
fn generation_script_is_idempotent_against_checked_in_fixture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let script = repo_root().join("scripts/s1_generate_tiny_fixture.sh");
    let status = Command::new(&script)
        .arg(temp.path())
        .current_dir(repo_root())
        .status()
        .expect("run tiny fixture generator");
    assert!(status.success());

    for file in [
        "train.bytes",
        "val.bytes",
        "manifest.toml",
        "generation_recipe.md",
    ] {
        let expected = std::fs::read(fixture_path(file)).expect("checked-in fixture file");
        let observed = std::fs::read(temp.path().join(file)).expect("generated fixture file");
        assert_eq!(observed, expected, "{file} changed under regeneration");
    }

    let generated =
        read_tinystories_manifest(temp.path().join("manifest.toml")).expect("generated manifest");
    assert_eq!(
        load_train_bytes(&generated).expect("generated train loads"),
        std::fs::read(temp.path().join("train.bytes")).expect("generated train")
    );
    assert_eq!(
        load_val_bytes(&generated).expect("generated val loads"),
        std::fs::read(temp.path().join("val.bytes")).expect("generated val")
    );
}

fn manifest_path() -> PathBuf {
    fixture_path("manifest.toml")
}

fn fixture_path(file: &str) -> PathBuf {
    repo_root().join(FIXTURE_DIR).join(file)
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn hash(value: &str) -> Hash256 {
    value.parse().expect("hash")
}

fn generated_prefix(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = InitRng::new(seed);
    (0..len)
        .map(|_| uniform_u64_inclusive(&mut rng, 0, 255) as u8)
        .collect()
}

fn story_count_from_separators(bytes: &[u8]) -> u64 {
    bytes
        .windows(SEPARATOR.len())
        .filter(|window| *window == SEPARATOR)
        .count() as u64
        + 1
}
