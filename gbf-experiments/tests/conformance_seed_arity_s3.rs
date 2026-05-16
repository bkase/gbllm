#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::fixture_envelope;
use gbf_artifact::SemanticCheckpoint;

#[test]
fn conformance_seed_arity_s3() {
    let envelope = fixture_envelope();

    assert_eq!(envelope.per_seed.len(), 5);
    let max_seed_tolerance = envelope
        .per_seed
        .iter()
        .map(|seed| seed.overall.tolerance)
        .fold(0.0_f32, f32::max);
    assert!(envelope.overall.tolerance >= max_seed_tolerance);

    for seed in &envelope.per_seed {
        assert!(
            seed.per_checkpoint
                .contains_key(&SemanticCheckpoint::PostLogits)
        );
        assert!(
            seed.per_checkpoint
                .contains_key(&SemanticCheckpoint::PostDecode)
        );
        assert!(
            seed.per_metric
                .keys()
                .any(|metric_id| metric_id.as_str().contains("max_abs_logit_diff"))
        );
    }
}
