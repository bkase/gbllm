#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use gbf_artifact::{
    Accumulator, ArtifactCore, ArtifactFeature, ArtifactManifest, CanonicalIntegerThenScale,
    CanonicalTensor, CanonicalTensorId, ComponentId, DecodeCapabilitySet, Dtype, EnvelopeGate,
    LexicalSpec_v1, LineageId, ManifestTimestamp, MetricGate, MetricId, ModelSpec_S3, PayloadRole,
    Q8_8Scale, QuantSpec_S3, QuantizationGapSummary, SeedConformanceEnvelope,
    SemanticCheckpointSchema, SequenceSemanticsSpec, WeightQuant,
};
use gbf_foundation::{ArtifactSchemaVersion, Hash256};

pub fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

pub fn id(value: &str) -> CanonicalTensorId {
    CanonicalTensorId::new(value).expect("valid canonical tensor id")
}

pub fn metric_id(value: &str) -> MetricId {
    MetricId::new(value).expect("valid metric id")
}

pub fn manifest() -> ArtifactManifest {
    ArtifactManifest {
        components: vec![],
        created_at: ManifestTimestamp(0),
        lineage: LineageId(hash(9)),
        manifest_self_hash: Hash256::ZERO,
        required_features: BTreeSet::from([ArtifactFeature::Ternary2Quant]),
        schema_version: ArtifactSchemaVersion { epoch: 3, minor: 0 },
        semantic_core_hash: hash(8),
    }
}

pub fn tensor(
    name: &str,
    dtype: Dtype,
    shape: Vec<u32>,
    payload_role: PayloadRole,
) -> CanonicalTensor {
    CanonicalTensor::new(
        id(name),
        dtype,
        shape,
        hash(name.as_bytes()[0]),
        payload_role,
    )
    .expect("valid tensor")
}

pub fn ternary_quant() -> WeightQuant {
    WeightQuant::Ternary2 {
        row_scale: Q8_8Scale(256),
        threshold: Q8_8Scale(32),
        accumulator: Accumulator::I32,
        reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
    }
}

pub fn quant_for(ids: &[CanonicalTensorId]) -> QuantSpec_S3 {
    QuantSpec_S3::new(
        ids.iter()
            .cloned()
            .map(|id| (id, ternary_quant()))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub fn artifact_core() -> ArtifactCore {
    let embedding = tensor(
        "tensor.embedding",
        Dtype::Ternary2,
        vec![80, 16],
        PayloadRole::DeployableWeight,
    );
    let linear = tensor(
        "tensor.linear.weight",
        Dtype::Ternary2,
        vec![16, 16],
        PayloadRole::DeployableWeight,
    );
    let scale = tensor(
        "tensor.linear.scale",
        Dtype::Q8_8,
        vec![16],
        PayloadRole::DeployableQuantParam,
    );
    let quant = quant_for(&[embedding.id.clone(), linear.id.clone()]);

    ArtifactCore::new(
        manifest(),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("toy0"),
        quant,
        SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
        vec![linear, scale, embedding],
        vec![],
        DecodeCapabilitySet::argmax_only(),
        None,
    )
    .expect("valid artifact core")
}

pub fn gate(tolerance: f32, passed: bool) -> EnvelopeGate {
    EnvelopeGate { tolerance, passed }
}

pub fn metric(
    value: f32,
    aggregation_kind: gbf_artifact::AggregationKind,
    passed: bool,
) -> MetricGate {
    MetricGate {
        value,
        aggregation_kind,
        passed,
    }
}

pub fn seed(seed: u64, metric_gate: MetricGate) -> SeedConformanceEnvelope {
    SeedConformanceEnvelope {
        seed,
        bundle_self_hash: hash(30 + seed as u8),
        artifact_self_hash: hash(40 + seed as u8),
        overall: gate(0.0, true),
        per_checkpoint: BTreeMap::from([
            (SemanticCheckpointSchema::PostLogits, gate(0.0, true)),
            (SemanticCheckpointSchema::PostDecode, gate(0.0, true)),
        ]),
        per_metric: BTreeMap::from([(metric_id("max_abs_logit_diff"), metric_gate)]),
    }
}

pub fn five_seeds(metric_gate: MetricGate) -> Vec<SeedConformanceEnvelope> {
    (0..5).map(|seed_id| seed(seed_id, metric_gate)).collect()
}

pub fn gap_summary() -> QuantizationGapSummary {
    QuantizationGapSummary {
        mean_per_token_max_abs_diff_phase_a: 0.0,
        mean_per_token_max_abs_diff_phase_d: 0.125,
        mean_per_token_kl: 0.25,
    }
}

#[allow(dead_code)]
pub fn component_id(value: &str) -> ComponentId {
    ComponentId(value.to_owned())
}
