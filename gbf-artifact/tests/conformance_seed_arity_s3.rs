mod artifact_b5_support;

use gbf_artifact::{AggregationKind, ConformanceEnvelope, ConformanceError};

use artifact_b5_support::{five_seeds, gap_summary, gate, hash, metric, seed};

#[test]
fn conformance_envelope_seed_arity_rejects_non_five_seed_inputs() {
    let err = ConformanceEnvelope::new(
        hash(1),
        vec![seed(
            0,
            metric(0.0, AggregationKind::PerTokenPerVocabRow, true),
        )],
        gate(0.0, true),
        gap_summary(),
    )
    .expect_err("one seed rejects");

    assert!(matches!(
        err,
        ConformanceError::SeedArityMismatch {
            expected: 5,
            actual: 1
        }
    ));
}

#[test]
fn conformance_envelope_five_seed_constructor_computes_self_hash() {
    let envelope = ConformanceEnvelope::new(
        hash(1),
        five_seeds(metric(0.0, AggregationKind::PerTokenPerVocabRow, true)),
        gate(0.0, true),
        gap_summary(),
    )
    .expect("five seeds construct");

    assert_eq!(
        envelope
            .compute_self_hash()
            .expect("conformance self hash computes"),
        envelope.conformance_self_hash
    );
}

#[test]
fn conformance_envelope_self_hash_is_deterministic_across_replays() {
    let baseline = ConformanceEnvelope::new(
        hash(1),
        five_seeds(metric(0.0, AggregationKind::PerTokenPerVocabRow, true)),
        gate(0.0, true),
        gap_summary(),
    )
    .expect("five seeds construct");

    for _ in 0..10 {
        let replay = ConformanceEnvelope::new(
            hash(1),
            five_seeds(metric(0.0, AggregationKind::PerTokenPerVocabRow, true)),
            gate(0.0, true),
            gap_summary(),
        )
        .expect("five seeds construct");
        assert_eq!(replay.conformance_self_hash, baseline.conformance_self_hash);
        assert_eq!(
            replay
                .compute_self_hash()
                .expect("conformance self hash computes"),
            baseline.conformance_self_hash
        );
    }
}

#[test]
fn conformance_envelope_public_json_shape_is_pinned() {
    let metric_gate = metric(0.0, AggregationKind::PerTokenPerVocabRow, true);
    let envelope = ConformanceEnvelope::new(
        hash(1),
        five_seeds(metric_gate),
        gate(0.0, true),
        gap_summary(),
    )
    .expect("five seeds construct");

    assert_eq!(
        serde_json::to_value(&envelope).expect("conformance serializes"),
        serde_json::json!({
            "schema": "s3_conformance.v1",
            "workload_self_hash": hash(1),
            "per_seed": five_seeds(metric_gate),
            "overall": gate(0.0, true),
            "quantization_gap_summary": gap_summary(),
            "real_owner_bead": "bd-35l3",
            "conformance_self_hash": envelope.conformance_self_hash,
        })
    );
}
