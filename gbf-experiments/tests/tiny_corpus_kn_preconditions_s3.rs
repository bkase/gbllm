#![cfg(feature = "s3")]

use std::path::{Path, PathBuf};

use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::KnEffectiveCounts;
use serde::Deserialize;

#[test]
fn tiny_corpus_s3_satisfies_kn_discount_preconditions() {
    let fixture_dir = workspace_root().join("fixtures/corpora/tiny_corpus_s3");
    let pinned = read_preconditions(&fixture_dir.join("preconditions.toml"));
    let train_bytes = std::fs::read(fixture_dir.join(&pinned.train_path))
        .expect("tiny S3 train fixture bytes read");
    let normalized = normalize_raw(&train_bytes).expect("tiny S3 train fixture normalizes");
    assert!(
        !normalized.dropped,
        "tiny S3 train fixture must survive charset_v1 normalization"
    );

    assert_eq!(pinned.schema, "s3_tiny_corpus_kn_preconditions.v1");
    assert_eq!(pinned.corpus_id, "tiny-corpus-s3-kn-smoke");
    assert_eq!(
        pinned.count_domain,
        "s3_kn_effective_counts.charset_v1_normalized_train_post"
    );
    assert_eq!(
        pinned
            .orders
            .iter()
            .map(|order| order.k)
            .collect::<Vec<_>>(),
        vec![2, 3, 4, 5],
        "precondition fixture pins exactly the effective KN orders"
    );

    let counts = KnEffectiveCounts::fit(&normalized.tokens)
        .expect("tiny S3 train fixture fits production KN effective counts");
    let observed = (2..=5)
        .map(|k| kn_precondition_counts(&counts, k))
        .collect::<Vec<_>>();

    for counts in &observed {
        eprintln!(
            "tiny_corpus_s3 k={} n_1={} n_2={} n_3={}",
            counts.k, counts.n_1, counts.n_2, counts.n_3
        );
        assert!(counts.n_1 > 0, "k={} n_1 must be non-zero", counts.k);
        assert!(counts.n_2 > 0, "k={} n_2 must be non-zero", counts.k);
        assert!(counts.n_3 > 0, "k={} n_3 must be non-zero", counts.k);
    }

    assert_eq!(
        observed, pinned.orders,
        "tiny corpus KN precondition counts must stay pinned"
    );
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preconditions {
    schema: String,
    corpus_id: String,
    train_path: String,
    count_domain: String,
    orders: Vec<OrderCounts>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct OrderCounts {
    k: usize,
    n_1: u64,
    n_2: u64,
    n_3: u64,
}

fn read_preconditions(path: &Path) -> Preconditions {
    let text = std::fs::read_to_string(path).expect("preconditions.toml reads");
    toml::from_str(&text).expect("preconditions.toml parses")
}

fn kn_precondition_counts(counts: &KnEffectiveCounts, k: usize) -> OrderCounts {
    let count_of_counts = counts
        .count_of_counts(k)
        .expect("production KN effective count-of-count table exists");
    OrderCounts {
        k,
        n_1: count_of_counts.get(&1).copied().unwrap_or(0),
        n_2: count_of_counts.get(&2).copied().unwrap_or(0),
        n_3: count_of_counts.get(&3).copied().unwrap_or(0),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
