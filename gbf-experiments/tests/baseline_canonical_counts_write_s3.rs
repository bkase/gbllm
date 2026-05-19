#![cfg(feature = "s3")]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{CanonicalKnCountsWrite, KnEffectiveCounts, NgramKey};

#[test]
fn canonical_counts_write_is_replay_deterministic() {
    let counts = oracle_counts();
    let expected = CanonicalKnCountsWrite::sha256(&counts);

    for _ in 0..10 {
        assert_eq!(CanonicalKnCountsWrite::sha256(&counts), expected);
        assert_eq!(
            gbf_foundation::sha256(CanonicalKnCountsWrite::encode(&counts)),
            expected
        );
    }
}

#[test]
fn canonical_counts_write_is_stable_after_unordered_source_reconstruction() {
    let counts = oracle_counts();
    let canonical = CanonicalKnCountsWrite::encode(&counts);
    // The encoder consumes canonical BTreeMap tables; this exercises rebuilding
    // those tables from an unordered intermediary inserted in a different order.
    let c2 = reconstruct_from_reversed_hashmap(counts.c2());
    let c3 = reconstruct_from_reversed_hashmap(counts.c3());
    let c4 = reconstruct_from_reversed_hashmap(counts.c4());
    let c5 = reconstruct_from_reversed_hashmap(counts.c5());

    assert_eq!(
        CanonicalKnCountsWrite::encode_tables(&c2, &c3, &c4, &c5),
        canonical
    );
}

fn reconstruct_from_reversed_hashmap<const K: usize>(
    table: &BTreeMap<NgramKey<K>, u64>,
) -> BTreeMap<NgramKey<K>, u64> {
    let mut hash = HashMap::new();
    for (key, count) in table.iter().rev() {
        hash.insert(*key, *count);
    }
    hash.into_iter().collect()
}

fn oracle_counts() -> KnEffectiveCounts {
    let root = workspace_root().join("fixtures/baselines/kn_oracle/train.bytes");
    let bytes = std::fs::read(root).expect("train fixture reads");
    let normalized = normalize_raw(&bytes).expect("train fixture normalizes");
    assert!(!normalized.dropped);
    KnEffectiveCounts::fit(&normalized.tokens).expect("effective counts fit")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
