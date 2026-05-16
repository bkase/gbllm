#![cfg(feature = "s3")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gbf_data::{read_tinystories_v2_manifest, verify_tinystories_v2_manifest};

#[test]
#[ignore = "nightly holdout contamination scan; run explicitly for B9 acceptance"]
fn held_out_chapter_prompt_windows_are_absent_from_train_post() {
    let manifest =
        read_tinystories_v2_manifest(tinystories_v2_manifest_path()).expect("manifest reads");
    let verification = verify_tinystories_v2_manifest(&manifest).expect("manifest hashes verify");

    let train = verification.train_post.as_slice();
    let chapter = verification.held_out_chapter.as_slice();
    let prompt_min = manifest.prompt_min_chars as usize;
    let prompt_max = manifest.prompt_max_chars as usize;
    assert!(prompt_min <= prompt_max);
    assert!(prompt_max <= chapter.len());

    assert_no_shared_window(train, chapter, prompt_min);

    for (index, offset) in manifest.prompt_offsets.iter().copied().enumerate() {
        let start = offset as usize;
        let end = start + prompt_max;
        assert!(
            end <= chapter.len(),
            "prompt offset {index} must have prompt_max chars available"
        );
        assert_no_shared_window(train, &chapter[start..end], prompt_min);
    }
}

fn assert_no_shared_window(train: &[u8], heldout: &[u8], window_len: usize) {
    assert!(window_len > 0);
    if train.len() < window_len || heldout.len() < window_len {
        return;
    }

    let train_hashes = rolling_window_index(train, window_len);
    for (heldout_start, heldout_window) in heldout.windows(window_len).enumerate() {
        let hash = rolling_hash(heldout_window);
        let Some(train_starts) = train_hashes.get(&hash) else {
            continue;
        };
        for train_start in train_starts {
            let train_window = &train[*train_start..*train_start + window_len];
            assert_ne!(
                train_window, heldout_window,
                "held-out chapter window at {heldout_start} appears in train_post at {train_start}"
            );
        }
    }
}

fn rolling_window_index(bytes: &[u8], window_len: usize) -> BTreeMap<u64, Vec<usize>> {
    let mut index = BTreeMap::<u64, Vec<usize>>::new();
    if bytes.len() < window_len {
        return index;
    }

    let mut hash = rolling_hash(&bytes[..window_len]);
    index.entry(hash).or_default().push(0);
    let highest_power = rolling_base_power(window_len - 1);

    for start in 1..=bytes.len() - window_len {
        let outgoing = normalized_byte(bytes[start - 1]).wrapping_mul(highest_power);
        hash = hash.wrapping_sub(outgoing);
        hash = hash.wrapping_mul(ROLLING_BASE);
        hash = hash.wrapping_add(normalized_byte(bytes[start + window_len - 1]));
        index.entry(hash).or_default().push(start);
    }

    index
}

fn rolling_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |hash, byte| {
        hash.wrapping_mul(ROLLING_BASE)
            .wrapping_add(normalized_byte(*byte))
    })
}

fn rolling_base_power(exponent: usize) -> u64 {
    (0..exponent).fold(1, |power, _| power.wrapping_mul(ROLLING_BASE))
}

fn normalized_byte(byte: u8) -> u64 {
    u64::from(byte) + 1
}

const ROLLING_BASE: u64 = 257;

fn tinystories_v2_manifest_path() -> PathBuf {
    workspace_root().join("fixtures/corpora/tinystories.v2.toml")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
