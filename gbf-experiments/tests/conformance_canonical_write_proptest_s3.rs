#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

use std::collections::BTreeMap;

use gbf_artifact::{
    AggregationKind, ConformanceEnvelope, EnvelopeGate, MetricGate, MetricId,
    QuantizationGapSummary, SeedConformanceEnvelope, SemanticCheckpoint,
    canonical_conformance_bytes,
};
use gbf_foundation::Hash256;
use proptest::prelude::*;

proptest! {
    #[test]
    fn conformance_canonical_write_round_trips(value in 0.0f32..10.0) {
        let envelope = envelope_with_metric_value(value);
        let bytes = canonical_conformance_bytes(&envelope).expect("canonical conformance bytes encode");
        let decoded: ConformanceEnvelope =
            serde_json::from_slice(&bytes).expect("canonical conformance json decodes");
        prop_assert_eq!(decoded, envelope);
    }

    #[test]
    fn conformance_metric_key_order_is_canonical(left in 0.0f32..10.0, right in 0.0f32..10.0) {
        let forward = envelope_with_order([(metric_id("prompt-00.phase_a.post_logits.step-0.max_abs_logit_diff"), left), (metric_id("prompt-00.phase_a.post_decode.step-0.argmax_match"), right)]);
        let reverse = envelope_with_order([(metric_id("prompt-00.phase_a.post_decode.step-0.argmax_match"), right), (metric_id("prompt-00.phase_a.post_logits.step-0.max_abs_logit_diff"), left)]);

        prop_assert_eq!(
            canonical_conformance_bytes(&forward).expect("forward canonicalizes"),
            canonical_conformance_bytes(&reverse).expect("reverse canonicalizes")
        );
    }
}

fn envelope_with_metric_value(value: f32) -> ConformanceEnvelope {
    envelope_with_order([(
        metric_id("prompt-00.phase_a.post_logits.step-0.max_abs_logit_diff"),
        value,
    )])
}

fn envelope_with_order<const N: usize>(metrics: [(MetricId, f32); N]) -> ConformanceEnvelope {
    let per_metric = metrics
        .into_iter()
        .map(|(metric_id, value)| {
            (
                metric_id,
                MetricGate {
                    value,
                    aggregation_kind: AggregationKind::PerTokenPerVocabRow,
                    passed: true,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut per_checkpoint = BTreeMap::new();
    per_checkpoint.insert(
        SemanticCheckpoint::PostLogits,
        EnvelopeGate {
            tolerance: 0.0,
            passed: true,
        },
    );
    per_checkpoint.insert(
        SemanticCheckpoint::PostDecode,
        EnvelopeGate {
            tolerance: 0.0,
            passed: true,
        },
    );
    let seed = SeedConformanceEnvelope {
        seed: 0,
        bundle_self_hash: Hash256::from_bytes([1; 32]),
        artifact_self_hash: Hash256::from_bytes([2; 32]),
        overall: EnvelopeGate {
            tolerance: 0.0,
            passed: true,
        },
        per_checkpoint,
        per_metric,
    };
    ConformanceEnvelope::new(
        Hash256::from_bytes([3; 32]),
        (0..5)
            .map(|index| SeedConformanceEnvelope {
                seed: index,
                bundle_self_hash: Hash256::from_bytes([index as u8 + 1; 32]),
                artifact_self_hash: Hash256::from_bytes([index as u8 + 11; 32]),
                ..seed.clone()
            })
            .collect(),
        EnvelopeGate {
            tolerance: 0.0,
            passed: true,
        },
        QuantizationGapSummary {
            mean_per_token_max_abs_diff_phase_a: 0.0,
            mean_per_token_max_abs_diff_phase_d: 0.0,
            mean_per_token_kl: 0.0,
        },
    )
    .expect("conformance envelope builds")
}

fn metric_id(value: &str) -> MetricId {
    MetricId::new(value).expect("metric id is valid")
}
