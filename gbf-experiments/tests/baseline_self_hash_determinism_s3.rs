#![cfg(feature = "s3")]

use std::path::{Path, PathBuf};

use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{KnBaselineInputs, s3_fit_kn5};
use gbf_foundation::Hash256;

#[test]
fn baseline_self_hash_is_deterministic_across_replays() {
    let mut observed = Vec::<Hash256>::new();
    for _ in 0..10 {
        let product = oracle_product();
        assert_eq!(
            product.baseline_self_hash,
            product.computed_self_hash().expect("self hash recomputes")
        );
        observed.push(product.baseline_self_hash);
    }

    assert!(observed.windows(2).all(|pair| pair[0] == pair[1]));
}

fn oracle_product() -> gbf_experiments::s3::baseline::KnBaselineProduct {
    let root = workspace_root().join("fixtures/baselines/kn_oracle");
    let train = normalize_file(&root.join("train.bytes"));
    let val = normalize_file(&root.join("eval.bytes"));
    s3_fit_kn5(KnBaselineInputs {
        train_post: train,
        val_post: val,
    })
    .expect("oracle baseline fits")
}

fn normalize_file(path: &Path) -> gbf_artifact::TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
