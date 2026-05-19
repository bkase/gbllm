#![cfg(feature = "s3")]

mod score_s3_support;

use gbf_artifact::TextCharSeq;
use gbf_experiments::s3::score::{S3_SCORE_CHUNK_SIZE, s3_score_bpc_char};
use proptest::prelude::*;
use score_s3_support::UniformEvaluator;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_text_char_seq_scores_finite_nonnegative(ids in text_ids(1, 2048)) {
        let seq = TextCharSeq::new(ids).unwrap();
        let product = s3_score_bpc_char(UniformEvaluator, &seq, S3_SCORE_CHUNK_SIZE);

        prop_assert!(product.bpc_char.get().is_finite());
        prop_assert!(product.bpc_char.get() >= 0.0);
        prop_assert!(product.log2_sum.is_finite());
        prop_assert!(product.log2_sum >= 0.0);
        prop_assert_eq!(product.char_count, seq.len() as u64);
    }

    #[test]
    fn same_seq_double_evaluator_is_byte_identical(ids in text_ids(1, 512)) {
        let seq = TextCharSeq::new(ids).unwrap();
        let first = s3_score_bpc_char(UniformEvaluator, &seq, S3_SCORE_CHUNK_SIZE);
        let second = s3_score_bpc_char(UniformEvaluator, &seq, S3_SCORE_CHUNK_SIZE);

        prop_assert_eq!(first, second);
    }
}

fn text_ids(min: usize, max: usize) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(prop_oneof![0_u8..=75, Just(79_u8)], min..=max)
}
