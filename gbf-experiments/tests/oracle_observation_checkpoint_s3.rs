#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod denotational_s3_support;

use denotational_s3_support::evaluate;
use gbf_artifact::VOCAB_SIZE;
use gbf_oracle::denotational::{Observation, RealDenotationalOracle, SemanticCheckpoint};

#[test]
fn oracle_observation_checkpoint_s3() {
    let product = evaluate(RealDenotationalOracle);

    let post_embedding = product
        .observations
        .iter()
        .find(|((_, checkpoint, _), _)| *checkpoint == SemanticCheckpoint::PostEmbedding)
        .map(|(_, observation)| observation)
        .expect("post_embedding observation exists");
    assert!(matches!(
        post_embedding,
        Observation::PostEmbedding { hidden_state: None }
    ));

    let post_logits = product
        .observations
        .iter()
        .find(|((_, checkpoint, _), _)| *checkpoint == SemanticCheckpoint::PostLogits)
        .map(|(_, observation)| observation)
        .expect("post_logits observation exists");
    match post_logits {
        Observation::PostLogits { logits } => assert_eq!(logits.len(), VOCAB_SIZE),
        other => panic!("unexpected post_logits observation {other:?}"),
    }

    let post_decode = product
        .observations
        .iter()
        .find(|((_, checkpoint, _), _)| *checkpoint == SemanticCheckpoint::PostDecode)
        .map(|(_, observation)| observation)
        .expect("post_decode observation exists");
    assert!(matches!(post_decode, Observation::PostDecode { token: 7 }));
}
