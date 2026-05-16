//! H6 adversarial artifact fixture for QuantSpec resolution tests.

use std::collections::{BTreeMap, BTreeSet};

use gbf_artifact::{
    Accumulator, ArtifactAux, ArtifactCore, ArtifactFeature, ArtifactManifest,
    ArtifactSchemaVersion, CanonicalIntegerThenScale, CanonicalTensor, CanonicalTensorId,
    ClassifierView, DecodeCapabilitySet, Dtype, LexicalSpec_v1, LineageId, ManifestTimestamp,
    ModelArtifact, ModelSpec_S3, PayloadRole, Q8_8Scale, QuantSpec_S3, SequenceSemanticsSpec,
    TextCharSeq, TiedEmbeddingAlias, WeightQuant,
};
use gbf_foundation::{Hash256, sha256};

use super::{OracleError, deliberate_name_resolver_logits, quant_spec_resolver_logits};

/// Canonical linear weight id used by the S3 dense-baseline artifact fixture.
pub const CANONICAL_LINEAR_WEIGHT_ID: &str = "linear_0_weight";
/// Shadow tensor id that a brittle name resolver would accidentally choose.
pub const SHADOW_LINEAR_WEIGHT_ID: &str = "linear_0_weight_naive_fp32";

/// Fixture artifact with canonical tensor naming only.
#[must_use]
pub fn canonical_naming_artifact_fixture() -> ModelArtifact {
    artifact_fixture(false)
}

/// H6 adversarial fixture containing both canonical and shadow linear tensors.
#[must_use]
pub fn adversarial_artifact_fixture() -> ModelArtifact {
    artifact_fixture(true)
}

/// Deliberate broken-S3 name resolver used only by H6 tests.
pub fn name_resolver_logits_for_fixture(
    artifact: &ModelArtifact,
    prompt: &TextCharSeq,
) -> Result<Vec<f32>, OracleError> {
    deliberate_name_resolver_logits(artifact, prompt)
}

/// Assert that the H6 fixture separates QuantSpec and name-resolution logits.
pub fn adversarial_fixture_is_structurally_separating() -> Result<bool, OracleError> {
    let artifact = adversarial_artifact_fixture();
    let prompt = separating_prompt();
    let quant = quant_spec_resolver_logits(&artifact, &prompt)?;
    let name = name_resolver_logits_for_fixture(&artifact, &prompt)?;
    Ok(max_abs_diff(&quant, &name) > 0.0)
}

/// Prompt chosen for deterministic H6 fixture separation.
#[must_use]
pub fn separating_prompt() -> TextCharSeq {
    TextCharSeq::new(vec![1, 4, 7, 10, 13, 16, 19]).expect("fixture prompt is valid text")
}

fn artifact_fixture(include_shadow: bool) -> ModelArtifact {
    let embedding_id = id("tensor.embedding");
    let linear_id = id(CANONICAL_LINEAR_WEIGHT_ID);
    let mut tensors = vec![
        tensor(
            embedding_id.clone(),
            Dtype::Ternary2,
            vec![80, 16],
            PayloadRole::DeployableWeight,
            b"h6:canonical-embedding",
        ),
        tensor(
            linear_id.clone(),
            Dtype::Ternary2,
            vec![16, 16],
            PayloadRole::DeployableWeight,
            b"h6:canonical-linear",
        ),
        tensor(
            id("tensor.linear_0_scale"),
            Dtype::Q8_8,
            vec![16],
            PayloadRole::DeployableQuantParam,
            b"h6:linear-scale",
        ),
    ];
    if include_shadow {
        tensors.push(tensor(
            id(SHADOW_LINEAR_WEIGHT_ID),
            Dtype::Fp32,
            vec![16, 16],
            PayloadRole::ReferenceFp32,
            b"h6:shadow-linear-naive-fp32",
        ));
    }

    let quant = QuantSpec_S3::new(BTreeMap::from([
        (embedding_id.clone(), ternary_quant(256)),
        (linear_id, ternary_quant(192)),
    ]));
    let core = ArtifactCore::new(
        manifest(sha256(if include_shadow {
            &b"h6-adversarial-artifact-semantic"[..]
        } else {
            &b"h6-canonical-artifact-semantic"[..]
        })),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny(if include_shadow {
            "artifact-oracle-h6-adversarial"
        } else {
            "artifact-oracle-canonical"
        }),
        quant,
        SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
        tensors,
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
    ModelArtifact::new(core, vec![], ArtifactAux::sparse(), None)
        .expect("model artifact constructs")
}

fn id(value: &str) -> CanonicalTensorId {
    CanonicalTensorId::new(value).expect("fixture tensor id is valid")
}

fn tensor(
    id: CanonicalTensorId,
    dtype: Dtype,
    shape: Vec<u32>,
    role: PayloadRole,
    salt: &[u8],
) -> CanonicalTensor {
    CanonicalTensor::new(id, dtype, shape, sha256(salt), role).expect("fixture tensor is valid")
}

fn ternary_quant(row_scale: u16) -> WeightQuant {
    WeightQuant::Ternary2 {
        row_scale: Q8_8Scale(row_scale),
        threshold: Q8_8Scale(32),
        accumulator: Accumulator::I32,
        reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
    }
}

fn manifest(semantic_core_hash: Hash256) -> ArtifactManifest {
    ArtifactManifest {
        components: vec![],
        created_at: ManifestTimestamp(0),
        lineage: LineageId(sha256(b"artifact-oracle-h6-lineage")),
        manifest_self_hash: Hash256::ZERO,
        required_features: BTreeSet::from([
            ArtifactFeature::Ternary2Quant,
            ArtifactFeature::LinearStateSequence,
        ]),
        schema_version: ArtifactSchemaVersion { epoch: 3, minor: 0 },
        semantic_core_hash,
    }
}

fn max_abs_diff(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0_f32, f32::max)
}
