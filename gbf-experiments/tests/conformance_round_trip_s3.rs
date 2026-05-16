#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::{canonical_bytes, fixture_envelope, write_envelope_if_requested};
use gbf_artifact::ConformanceEnvelope;

#[test]
fn conformance_round_trip_s3() {
    let envelope = fixture_envelope();
    write_envelope_if_requested(&envelope);
    let bytes = canonical_bytes(&envelope).expect("canonical conformance bytes encode");
    let decoded: ConformanceEnvelope =
        serde_json::from_slice(&bytes).expect("canonical conformance json decodes");

    assert_eq!(decoded, envelope);
    assert_eq!(
        decoded
            .compute_self_hash()
            .expect("decoded self hash computes"),
        envelope.conformance_self_hash
    );
    for seed in &decoded.per_seed {
        let keys = seed.per_metric.keys().collect::<Vec<_>>();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert!(seed.per_metric.values().all(|metric| {
            metric.aggregation_kind == gbf_artifact::AggregationKind::PerTokenPerVocabRow
        }));
    }
}
