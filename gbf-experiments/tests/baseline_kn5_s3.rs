#![cfg(feature = "s3")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gbf_artifact::TextCharSeq;
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{
    KnBaselineInputs, KnConditionalModel, KnEffectiveCounts, NgramKey, c_continuation_counts,
    c5_raw_counts, count_of_counts, p_kn_4, p_kn_5, s3_fit_kn5,
};
use gbf_foundation::Hash256;
use serde::Deserialize;

#[test]
fn kn5_oracle_fixture_matches_expected_bpc_discounts_and_counts() {
    let fixture = oracle_fixture();
    let product = s3_fit_kn5(KnBaselineInputs {
        train_post: fixture.train.clone(),
        val_post: fixture.val.clone(),
    })
    .expect("KN baseline fits oracle fixture");

    assert_eq!(product.schema, "s3_baseline_kn5.v1");
    assert_eq!(
        product.train_post_sha256,
        fixture.expected.train_post_sha256
    );
    assert_eq!(product.val_post_sha256, fixture.expected.val_post_sha256);
    assert_close(product.bpc_kn1_val, fixture.expected.bpc_kn1_val);
    assert_close(product.bpc_kn2_val, fixture.expected.bpc_kn2_val);
    assert_close(product.bpc_kn3_val, fixture.expected.bpc_kn3_val);
    assert_close(product.bpc_kn4_val, fixture.expected.bpc_kn4_val);
    assert_close(product.bpc_kn5_val, fixture.expected.bpc_kn5_val);
    assert_eq!(
        product.counts_blob_sha256,
        fixture.expected.counts_blob_sha256
    );
    assert_eq!(
        product.baseline_self_hash,
        fixture.expected.baseline_self_hash
    );
    assert_eq!(product.counts_summary, fixture.expected.counts_summary);

    let counts = KnEffectiveCounts::fit(&fixture.train).expect("effective counts fit");
    assert_eq!(counts.p1_left_continuation_count(id('a')), 6);
    assert_eq!(counts.p1_left_continuation_count(id('f')), 6);
    assert_eq!(counts.p1_left_continuation_count(75), 1);
    assert_eq!(counts.p1_denominator(), 37);
    assert_eq!(
        c_continuation_counts::<2>(&fixture.train)
            .get(&NgramKey::new([id('e'), id('c')]))
            .copied(),
        Some(6)
    );
    assert_eq!(
        c_continuation_counts::<3>(&fixture.train)
            .get(&NgramKey::new([id('e'), id('c'), id('c')]))
            .copied(),
        Some(4)
    );
    assert_eq!(
        c_continuation_counts::<4>(&fixture.train)
            .get(&NgramKey::new([id('c'), id('c'), id('a'), id('a')]))
            .copied(),
        Some(1)
    );
    assert_eq!(
        c5_raw_counts(&fixture.train)
            .get(&NgramKey::new([
                id('a'),
                id('a'),
                id('f'),
                id('c'),
                id('f')
            ]))
            .copied(),
        Some(3)
    );

    for order in &fixture.expected.orders {
        let observed_coc = match order.k {
            2 => count_of_counts(counts.c2()),
            3 => count_of_counts(counts.c3()),
            4 => count_of_counts(counts.c4()),
            5 => count_of_counts(counts.c5()),
            other => panic!("unexpected order {other}"),
        };
        assert_eq!(observed_coc, order.count_of_counts);
        let discounts = product
            .discounts
            .get(&order.k)
            .expect("discount order present");
        assert_close(discounts.y_k, order.y_k);
        assert_close(discounts.d_1, order.d_1);
        assert_close(discounts.d_2, order.d_2);
        assert_close(discounts.d_3p, order.d_3p);
        assert!(discounts.y_k > 0.0 && discounts.y_k < 1.0);
        assert!((0.0..=1.0).contains(&discounts.d_1));
        assert!((0.0..=2.0).contains(&discounts.d_2));
        assert!((0.0..=3.0).contains(&discounts.d_3p));
    }

    let model = KnConditionalModel::new(counts, product.discounts.clone());
    let unseen_context = [75, 75, 75, 75];
    let target = id('a');
    assert_eq!(
        p_kn_5(&model, unseen_context, target).expect("P5 backs off"),
        p_kn_4(&model, [75, 75, 75], target).expect("P4 suffix probability")
    );
}

#[derive(Debug)]
struct OracleFixture {
    train: TextCharSeq,
    val: TextCharSeq,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    schema: String,
    train_post_sha256: Hash256,
    val_post_sha256: Hash256,
    bpc_kn1_val: f64,
    bpc_kn2_val: f64,
    bpc_kn3_val: f64,
    bpc_kn4_val: f64,
    bpc_kn5_val: f64,
    counts_blob_sha256: Hash256,
    baseline_self_hash: Hash256,
    counts_summary: gbf_experiments::s3::baseline::CountsSummary,
    p1_left_continuations: BTreeMap<String, u64>,
    orders: Vec<ExpectedOrder>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedOrder {
    k: u64,
    count_of_counts: BTreeMap<u64, u64>,
    y_k: f64,
    d_1: f64,
    d_2: f64,
    d_3p: f64,
}

fn oracle_fixture() -> OracleFixture {
    let root = workspace_root().join("fixtures/baselines/kn_oracle");
    let train = normalize_file(&root.join("train.bytes"));
    let val = normalize_file(&root.join("eval.bytes"));
    let expected_text =
        std::fs::read_to_string(root.join("expected.toml")).expect("expected fixture reads");
    let expected: Expected = toml::from_str(&expected_text).expect("expected fixture parses");
    assert_eq!(expected.schema, "s3_kn_oracle_expected.v1");
    assert_eq!(expected.p1_left_continuations["denominator"], 37);
    OracleFixture {
        train,
        val,
        expected,
    }
}

fn normalize_file(path: &Path) -> TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn id(ch: char) -> u8 {
    assert!(ch.is_ascii_lowercase());
    26 + (ch as u8 - b'a')
}

fn assert_close(observed: f64, expected: f64) {
    assert!(
        (observed - expected).abs() <= 1.0e-12,
        "observed={observed:.17} expected={expected:.17}"
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
