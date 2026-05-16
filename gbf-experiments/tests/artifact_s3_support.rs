#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use gbf_artifact::{
    Accumulator, ArtifactAux, ArtifactCore, ArtifactFeature, ArtifactManifest,
    CanonicalIntegerThenScale, CanonicalTensor, CanonicalTensorId, ClassifierView,
    DecodeCapabilitySet, Dtype, GoldenVectorId, GoldenVectorRef, LexicalSpec_v1, LineageId,
    ManifestTimestamp, ModelArtifact, ModelSpec_S3, PayloadRole, Q8_8Scale, QuantSpec_S3,
    SequenceSemanticsSpec, TiedEmbeddingAlias, WeightQuant,
};
use gbf_experiments::s3::artifact::{
    ArtifactExportInputs, ArtifactExportProduct, s3_export_model_artifact,
};
use gbf_foundation::{ArtifactSchemaVersion, Hash256, sha256};
use gbf_train::export_visitor::{ArtifactExportModel, ExportVisitor, ExportVisitorError};
use gbf_train::student::{
    FrozenStudent, HardTernaryStudentModel, StudentStorageFingerprint, StudentWeightFingerprint,
    freeze_student_as_artifact,
};

static NEXT_STORAGE_ID: AtomicUsize = AtomicUsize::new(90_000);

#[derive(Debug, PartialEq, Eq)]
pub struct ToyArtifactStudent {
    seed: u64,
    requires_grad: bool,
    storage_identity: usize,
    classifier_view: ClassifierView,
    missing_linear_quant: bool,
}

impl ToyArtifactStudent {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            requires_grad: true,
            storage_identity: next_storage_id(),
            classifier_view: ClassifierView::SameTensor,
            missing_linear_quant: false,
        }
    }

    pub fn with_classifier_view(seed: u64, classifier_view: ClassifierView) -> Self {
        Self {
            classifier_view,
            ..Self::new(seed)
        }
    }

    pub fn missing_linear_quant(seed: u64) -> Self {
        Self {
            missing_linear_quant: true,
            ..Self::new(seed)
        }
    }

    fn fingerprint_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::from("artifact-s3-support:toy-student:");
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        bytes.push(classifier_view_tag(self.classifier_view));
        for tensor in base_tensors(self.seed) {
            bytes.extend_from_slice(tensor.id.as_str().as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(tensor.payload_sha.as_bytes());
            bytes.push(0xff);
        }
        bytes
    }
}

impl Clone for ToyArtifactStudent {
    fn clone(&self) -> Self {
        Self {
            seed: self.seed,
            requires_grad: self.requires_grad,
            storage_identity: next_storage_id(),
            classifier_view: self.classifier_view,
            missing_linear_quant: self.missing_linear_quant,
        }
    }
}

impl HardTernaryStudentModel for ToyArtifactStudent {
    fn detach_for_student(&mut self) {
        self.requires_grad = false;
        self.storage_identity = next_storage_id();
    }

    fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
        StudentWeightFingerprint::new(self.fingerprint_bytes()).expect("fixture fingerprint")
    }

    fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
        let mut bytes = Vec::from("artifact-s3-support:toy-storage:");
        bytes.extend_from_slice(&self.fingerprint_bytes());
        StudentStorageFingerprint::new(bytes).expect("fixture storage fingerprint")
    }

    fn student_storage_identity(&self) -> usize {
        self.storage_identity
    }

    fn student_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

impl ArtifactExportModel for ToyArtifactStudent {
    fn artifact_seed(&self) -> u64 {
        self.seed
    }

    fn artifact_model(&self) -> ModelSpec_S3 {
        ModelSpec_S3::tiny(format!("toy-artifact-student-{}", self.seed))
    }

    fn artifact_quant_spec(&self) -> QuantSpec_S3 {
        quant_spec(self.missing_linear_quant)
    }

    fn artifact_sequence_semantics(&self) -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::linear_state(4).expect("valid sequence spec")
    }

    fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError> {
        Ok(base_tensors(self.seed))
    }

    fn artifact_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        Some(TiedEmbeddingAlias::new(
            id("tensor.embedding"),
            id("tensor.embedding"),
            true,
            self.classifier_view,
        ))
    }
}

pub fn frozen_student(seed: u64) -> FrozenStudent<ToyArtifactStudent> {
    freeze_student_as_artifact(&ToyArtifactStudent::new(seed)).expect("toy student freezes")
}

pub fn frozen_with_view(seed: u64, view: ClassifierView) -> FrozenStudent<ToyArtifactStudent> {
    freeze_student_as_artifact(&ToyArtifactStudent::with_classifier_view(seed, view))
        .expect("toy student freezes")
}

pub fn frozen_missing_quant(seed: u64) -> FrozenStudent<ToyArtifactStudent> {
    freeze_student_as_artifact(&ToyArtifactStudent::missing_linear_quant(seed))
        .expect("toy student freezes")
}

pub fn export_product(seed: u64) -> ArtifactExportProduct {
    let frozen = frozen_student(seed);
    export_product_from(&frozen)
}

pub fn export_product_from(frozen: &FrozenStudent<ToyArtifactStudent>) -> ArtifactExportProduct {
    s3_export_model_artifact(ArtifactExportInputs::new(frozen, ExportVisitor::pinned()))
        .expect("toy artifact export succeeds")
}

pub fn id(value: &str) -> CanonicalTensorId {
    CanonicalTensorId::new(value).expect("valid tensor id")
}

pub fn tensor(
    name: &str,
    dtype: Dtype,
    shape: Vec<u32>,
    role: PayloadRole,
    salt: u8,
) -> CanonicalTensor {
    CanonicalTensor::new(
        id(name),
        dtype,
        shape,
        Hash256::from_bytes([salt; 32]),
        role,
    )
    .expect("valid tensor")
}

pub fn ternary_quant(row_scale: u16) -> WeightQuant {
    WeightQuant::Ternary2 {
        row_scale: Q8_8Scale(row_scale),
        threshold: Q8_8Scale(32),
        accumulator: Accumulator::I32,
        reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
    }
}

pub fn quant_for(ids: &[CanonicalTensorId]) -> QuantSpec_S3 {
    QuantSpec_S3::new(
        ids.iter()
            .cloned()
            .enumerate()
            .map(|(index, id)| (id, ternary_quant(128 + index as u16)))
            .collect::<BTreeMap<_, _>>(),
    )
}

pub fn artifact_from_tensors(
    tensors: Vec<CanonicalTensor>,
    quant: QuantSpec_S3,
    aux: ArtifactAux,
    alias: Option<TiedEmbeddingAlias>,
) -> ModelArtifact {
    let core = ArtifactCore::new(
        manifest(sha256(b"artifact-s3-support:semantic")),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("deployable-bytes-fixture"),
        quant,
        SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
        tensors,
        vec![],
        DecodeCapabilitySet::argmax_only(),
        alias,
    )
    .expect("artifact core constructs");
    ModelArtifact::new(core, vec![], aux, None).expect("model artifact constructs")
}

pub fn sparse_aux_with_sidecar(salt: u8) -> ArtifactAux {
    let mut aux = ArtifactAux::sparse();
    aux.golden_vectors.push(GoldenVectorRef {
        id: GoldenVectorId(format!("sidecar-{salt}")),
        manifest_hash: Hash256::from_bytes([salt; 32]),
    });
    aux
}

fn base_tensors(seed: u64) -> Vec<CanonicalTensor> {
    vec![
        seeded_tensor(
            "tensor.embedding",
            Dtype::Ternary2,
            vec![80, 16],
            PayloadRole::DeployableWeight,
            seed,
        ),
        seeded_tensor(
            "tensor.linear.weight",
            Dtype::Ternary2,
            vec![16, 16],
            PayloadRole::DeployableWeight,
            seed,
        ),
        seeded_tensor(
            "tensor.linear.scale",
            Dtype::Q8_8,
            vec![16],
            PayloadRole::DeployableQuantParam,
            seed,
        ),
    ]
}

fn seeded_tensor(
    name: &str,
    dtype: Dtype,
    shape: Vec<u32>,
    role: PayloadRole,
    seed: u64,
) -> CanonicalTensor {
    let mut preimage = Vec::from("artifact-s3-support:tensor:");
    preimage.extend_from_slice(name.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&seed.to_le_bytes());
    CanonicalTensor::new(id(name), dtype, shape, sha256(preimage), role).expect("valid tensor")
}

fn quant_spec(missing_linear: bool) -> QuantSpec_S3 {
    let mut quant = BTreeMap::from([(id("tensor.embedding"), ternary_quant(256))]);
    if !missing_linear {
        quant.insert(id("tensor.linear.weight"), ternary_quant(192));
    }
    QuantSpec_S3::new(quant)
}

fn manifest(semantic_core_hash: Hash256) -> ArtifactManifest {
    ArtifactManifest {
        components: vec![],
        created_at: ManifestTimestamp(0),
        lineage: LineageId(sha256(b"artifact-s3-support:lineage")),
        manifest_self_hash: Hash256::ZERO,
        required_features: BTreeSet::from([
            ArtifactFeature::Ternary2Quant,
            ArtifactFeature::LinearStateSequence,
        ]),
        schema_version: ArtifactSchemaVersion { epoch: 3, minor: 0 },
        semantic_core_hash,
    }
}

fn next_storage_id() -> usize {
    NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed)
}

const fn classifier_view_tag(view: ClassifierView) -> u8 {
    match view {
        ClassifierView::SameTensor => 0,
        ClassifierView::TransposedView => 1,
    }
}
