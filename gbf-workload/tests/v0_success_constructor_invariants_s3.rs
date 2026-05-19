#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

use gbf_artifact::{LexicalError, TextCharSeq};
use gbf_foundation::{Hash256, sha256};
use gbf_workload::{
    PromptCase, V0_SUCCESS_HELD_OUT_CHAPTER_SHA, V0_SUCCESS_PROMPT_COUNT, WorkloadError,
};

#[test]
fn prompt_arity_mismatch_rejects() {
    let mut manifest = v0_success_s3_support::parse_v0_success_unverified();
    manifest.prompts.pop();

    assert!(matches!(
        manifest.validate_invariants(),
        Err(WorkloadError::PromptArityMismatch {
            expected: V0_SUCCESS_PROMPT_COUNT,
            observed: 7,
        })
    ));
}

#[test]
fn prompt_length_bounds_reject() {
    for (len, expected_len) in [(63, 63), (129, 129)] {
        let mut manifest = v0_success_s3_support::parse_v0_success_unverified();
        manifest.prompts[0].prompt_chars =
            TextCharSeq::new(vec![0; len]).expect("fixture ids are lexical text");

        assert!(matches!(
            manifest.validate_invariants(),
            Err(WorkloadError::PromptLengthOutOfBounds {
                len,
                min: 64,
                max: 128,
                ..
            }) if len == expected_len
        ));
    }
}

#[test]
fn prompt_constructor_propagates_reserved_id_76() {
    let chapter_sha = V0_SUCCESS_HELD_OUT_CHAPTER_SHA
        .parse()
        .expect("pinned chapter hash parses");

    assert!(matches!(
        PromptCase::new("bad-reserved", vec![76], chapter_sha),
        Err(WorkloadError::Lexical(LexicalError::ReservedId76 {
            position: 0
        }))
    ));
}

#[test]
fn held_out_chapter_sha_mismatch_rejects() {
    let mut manifest = v0_success_s3_support::parse_v0_success_unverified();
    manifest.prompts[0].held_out_chapter_sha = Hash256::from_bytes([9; 32]);

    assert!(matches!(
        manifest.validate_invariants(),
        Err(WorkloadError::ChapterShaMismatch { .. })
    ));
}

#[test]
fn held_out_chapter_sha_matches_corpus_manifest_fixture() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-workload has workspace parent");
    let manifest_path = root.join("fixtures/corpora/tinystories.v2.toml");
    let manifest_text =
        std::fs::read_to_string(&manifest_path).expect("tinystories.v2 manifest reads");
    let manifest: toml::Value =
        toml::from_str(&manifest_text).expect("tinystories.v2 manifest parses as TOML");

    let chapter_rel = manifest["held_out_chapter_path"]
        .as_str()
        .expect("held_out_chapter_path is a string");
    assert_eq!(
        chapter_rel,
        "accelerando_v0_success_fixture/heldout_chapter.txt"
    );

    let chapter = std::fs::read(root.join("fixtures/corpora").join(chapter_rel))
        .expect("held-out Accelerando chapter fixture reads");
    let computed = sha256(&chapter);
    let workload_pinned: Hash256 = V0_SUCCESS_HELD_OUT_CHAPTER_SHA
        .parse()
        .expect("workload chapter hash parses");
    let manifest_pinned: Hash256 = manifest["chapter_sha256"]
        .as_str()
        .expect("chapter_sha256 is a string")
        .parse()
        .expect("manifest chapter hash parses");

    assert_eq!(computed, workload_pinned);
    assert_eq!(manifest_pinned, workload_pinned);
    assert_eq!(
        manifest["chapter_char_count"]
            .as_integer()
            .expect("chapter_char_count is an integer"),
        chapter.len() as i64
    );
}
