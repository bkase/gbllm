#![cfg(feature = "s3")]

mod score_s3_support;

use gbf_experiments::s3::score::{S3_SCORE_CHUNK_SIZE, ScoreError, try_s3_score_bpc_char};
use score_s3_support::{
    PromptWideSoftmaxShapeEvaluator, ShortLogitsEvaluator, UniformEvaluator, repeated_a,
};

#[test]
fn evaluator_output_logits_are_one_vocab_row_per_token() {
    let val = repeated_a(3);
    let product = try_s3_score_bpc_char(UniformEvaluator, &val, S3_SCORE_CHUNK_SIZE).unwrap();

    assert_eq!(product.char_count, 3);
}

#[test]
fn rejects_short_or_prompt_wide_logits_before_scoring() {
    let val = repeated_a(2);

    assert!(matches!(
        try_s3_score_bpc_char(ShortLogitsEvaluator, &val, S3_SCORE_CHUNK_SIZE),
        Err(ScoreError::LogitsWrongLength { len, expected }) if len == 79 && expected == 80
    ));
    assert!(matches!(
        try_s3_score_bpc_char(PromptWideSoftmaxShapeEvaluator, &val, S3_SCORE_CHUNK_SIZE),
        Err(ScoreError::LogitsWrongLength { len, expected }) if len == 160 && expected == 80
    ));
}
