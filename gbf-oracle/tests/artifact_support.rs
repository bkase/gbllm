#![allow(dead_code)]

use std::collections::BTreeMap;

use gbf_artifact::{
    Accumulator, ArtifactAux, ArtifactCore, CanonicalIntegerThenScale, CanonicalTensor,
    CanonicalTensorId, ClassifierView, DecodeCapabilitySet, Dtype, LexicalSpec_v1, ModelArtifact,
    ModelSpec_S3, PayloadRole, Q8_8Scale, QuantSpec_S3, SequenceSemanticsSpec, TextCharSeq,
    TiedEmbeddingAlias, UNK_ID, WeightQuant,
};
use gbf_foundation::{Hash256, sha256};

pub fn fixture_artifact() -> ModelArtifact {
    let embedding_id = id("tensor.embedding");
    let linear_id = id("linear_0_weight");
    let core = ArtifactCore::new(
        ModelArtifact::fixture_manifest(29, sha256(b"artifact-oracle-test-semantic")),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("artifact-oracle-test"),
        QuantSpec_S3::new(BTreeMap::from([
            (embedding_id.clone(), ternary_quant(256)),
            (linear_id.clone(), ternary_quant(192)),
        ])),
        SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
        vec![
            tensor(
                embedding_id.clone(),
                Dtype::Ternary2,
                vec![80, 16],
                PayloadRole::DeployableWeight,
                1,
            ),
            tensor(
                linear_id,
                Dtype::Ternary2,
                vec![16, 16],
                PayloadRole::DeployableWeight,
                2,
            ),
            tensor(
                id("tensor.linear_0_scale"),
                Dtype::Q8_8,
                vec![16],
                PayloadRole::DeployableQuantParam,
                3,
            ),
        ],
        vec![],
        DecodeCapabilitySet::argmax_only(),
        Some(TiedEmbeddingAlias::new(
            embedding_id.clone(),
            embedding_id,
            true,
            ClassifierView::SameTensor,
        )),
    )
    .expect("artifact core constructs");
    ModelArtifact::new(core, vec![], ArtifactAux::sparse(), None).expect("artifact constructs")
}

pub fn fixture_prompt() -> TextCharSeq {
    TextCharSeq::new(vec![1, 2, 3, 4, 5]).expect("fixture prompt validates")
}

pub fn eos_trigger_prompt() -> TextCharSeq {
    TextCharSeq::new(vec![UNK_ID]).expect("eos trigger prompt validates")
}

fn id(value: &str) -> CanonicalTensorId {
    CanonicalTensorId::new(value).expect("fixture tensor id validates")
}

fn tensor(
    id: CanonicalTensorId,
    dtype: Dtype,
    shape: Vec<u32>,
    role: PayloadRole,
    salt: u8,
) -> CanonicalTensor {
    CanonicalTensor::new(id, dtype, shape, Hash256::from_bytes([salt; 32]), role)
        .expect("fixture tensor validates")
}

fn ternary_quant(row_scale: u16) -> WeightQuant {
    WeightQuant::Ternary2 {
        row_scale: Q8_8Scale(row_scale),
        threshold: Q8_8Scale(32),
        accumulator: Accumulator::I32,
        reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
    }
}
