#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod score_s3_support;

use gbf_experiments::s3::score::Evaluator;
use gbf_oracle::scorers::{ArtifactScorer, artifact_logits_from_core};
use score_s3_support::artifact_fixture;

#[test]
fn artifact_scorer_matches_canonical_artifact_row_evaluator_bitwise() {
    let artifact = artifact_fixture();
    let scorer = ArtifactScorer::new(&artifact);
    let prefix = [3, 1, 4, 1];

    let observed = scorer.forward(&prefix, 4).logits;
    let expected = artifact_logits_from_core(&artifact, &prefix);

    assert_eq!(observed, expected);
}
