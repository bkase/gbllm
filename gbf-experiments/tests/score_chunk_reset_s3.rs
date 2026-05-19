#![cfg(feature = "s3")]

mod score_s3_support;

use gbf_artifact::{TextCharSeq, evaluate_reference_program};
use gbf_experiments::s3::score::{Evaluator, KnScorer, S3_SCORE_CHUNK_SIZE, s3_score_bpc_char};
use gbf_oracle::scorers::{ArtifactScorer, ReferenceScorer};
use score_s3_support::{
    ContextSensitiveEvaluator, TARGET_A, artifact_fixture, kn_product_for_val,
    predictable_a_bundle, repeated_a,
};

#[test]
fn chunk_size_controls_reset_boundaries() {
    let val = repeated_a(256);

    let two_chunks = s3_score_bpc_char(ContextSensitiveEvaluator, &val, S3_SCORE_CHUNK_SIZE);
    let one_chunk = s3_score_bpc_char(ContextSensitiveEvaluator, &val, 256);

    assert_ne!(two_chunks.bpc_char, one_chunk.bpc_char);
    assert!(
        two_chunks.bpc_char.get() > one_chunk.bpc_char.get(),
        "extra reset should force an additional empty-context score"
    );
}

#[test]
fn kn_first_char_uses_p_kn_1_empty_context() {
    let val = repeated_a(8);
    let (train, product) = kn_product_for_val(val);
    let scorer = KnScorer::from_product_and_train(&product, &train).unwrap();

    let output = scorer.forward(&[], usize::from(TARGET_A));
    let probability = output.target_logprob.exp();

    assert!((probability - 6.0 / 37.0).abs() <= 1.0e-12);
}

#[test]
fn reference_and_artifact_empty_context_rows_are_canonical() {
    let bundle = predictable_a_bundle();
    let reference = ReferenceScorer::new(&bundle);
    let empty = TextCharSeq::new(Vec::new()).unwrap();
    let direct = evaluate_reference_program(&bundle, &empty, &()).logits;

    assert_eq!(reference.forward(&[], usize::from(TARGET_A)).logits, direct);

    let artifact = artifact_fixture();
    let artifact_scorer = ArtifactScorer::new(&artifact);
    assert_eq!(
        artifact_scorer.forward(&[], usize::from(TARGET_A)).logits,
        artifact_scorer.forward_logits(&[])
    );
}
