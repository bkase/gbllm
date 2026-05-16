#![cfg(feature = "s3")]

mod score_s3_support;

use gbf_experiments::s3::score::{S3_SCORE_CHUNK_SIZE, s3_score_bpc_char};
use score_s3_support::{UniformEvaluator, repeated_a};
use serde_json::json;

#[test]
fn same_sequence_and_evaluator_replay_byte_identical_score_product() {
    let val = repeated_a(257);
    let first = s3_score_bpc_char(UniformEvaluator, &val, S3_SCORE_CHUNK_SIZE);

    assert_eq!(first.schema, "s3_score.v1");
    assert_eq!(first.char_count, val.len() as u64);
    assert!((first.bpc_char.get() - 80.0_f64.log2()).abs() <= 1.0e-12);
    assert_eq!(first.score_self_hash, first.computed_self_hash().unwrap());
    assert_eq!(
        serde_json::to_value(&first).expect("s3 score JSON shape"),
        json!({
            "schema": "s3_score.v1",
            "scorer_kind": "ReferenceScorer",
            "chunk_size": S3_SCORE_CHUNK_SIZE as u64,
            "bpc_char": first.bpc_char.get(),
            "char_count": first.char_count,
            "log2_sum": first.log2_sum,
            "score_self_hash": first.score_self_hash.to_string(),
        })
    );

    for _ in 0..10 {
        assert_eq!(
            s3_score_bpc_char(UniformEvaluator, &val, S3_SCORE_CHUNK_SIZE),
            first
        );
    }
}
