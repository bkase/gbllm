mod artifact_b5_support;

use gbf_artifact::{AggregationKind, ConformanceEnvelope, ConformanceError};

use artifact_b5_support::{five_seeds, gap_summary, gate, hash, metric};

#[test]
fn conformance_aggregation_kind_prompt_wide_softmax_rejects_at_canonical_write() {
    let envelope = ConformanceEnvelope::new(
        hash(1),
        five_seeds(metric(
            7.0,
            AggregationKind::PromptWideSoftmaxForbidden,
            false,
        )),
        gate(7.0, false),
        gap_summary(),
    )
    .expect("schema can carry forbidden marker before canonical write");

    let err = envelope
        .canonical_bytes()
        .expect_err("canonical conformance write rejects forbidden aggregation");

    assert!(matches!(
        err,
        ConformanceError::ForbiddenAggregationKind { seed: 0, .. }
    ));
}

#[test]
fn conformance_aggregation_kind_per_token_per_vocab_row_canonicalizes() {
    let envelope = ConformanceEnvelope::new(
        hash(1),
        five_seeds(metric(0.0, AggregationKind::PerTokenPerVocabRow, true)),
        gate(0.0, true),
        gap_summary(),
    )
    .expect("valid conformance envelope");

    let canonical = envelope
        .canonical_bytes()
        .expect("valid conformance envelope canonicalizes");
    assert!(!canonical.is_empty());
}
