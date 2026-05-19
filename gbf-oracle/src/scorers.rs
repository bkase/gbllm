//! S3 scorer wrappers hosted outside gbf-experiments.

use gbf_artifact::{
    CharId, Dtype, ModelArtifact, PayloadRole, ReferenceModelBundle, TextCharSeq, VOCAB_SIZE,
    WeightQuant, evaluate_reference_program,
};
use gbf_foundation::sha256;

/// Full-precision scorer over an exported `ReferenceModelBundle`.
#[derive(Debug)]
pub struct ReferenceScorer<'a> {
    bundle: &'a ReferenceModelBundle,
    reset_count: u64,
}

impl<'a> ReferenceScorer<'a> {
    /// Wrap a reference bundle for teacher-forced S3 scoring.
    #[must_use]
    pub const fn new(bundle: &'a ReferenceModelBundle) -> Self {
        Self {
            bundle,
            reset_count: 0,
        }
    }

    /// Borrow the wrapped bundle.
    #[must_use]
    pub const fn bundle(&self) -> &ReferenceModelBundle {
        self.bundle
    }

    /// Number of reset notifications observed by this wrapper.
    #[must_use]
    pub const fn reset_count(&self) -> u64 {
        self.reset_count
    }

    /// Evaluate one per-token vocab row for the provided reset-chunk prefix.
    #[must_use]
    pub fn forward_logits(&self, prefix: &[CharId]) -> Vec<f32> {
        let prompt = TextCharSeq::new(prefix.to_vec()).expect("score prefix contains text ids");
        evaluate_reference_program(self.bundle, &prompt, &()).logits
    }

    /// Reset provider-owned state between S3 chunks.
    pub fn reset_state(&mut self) {
        self.reset_count += 1;
    }
}

/// Quantized artifact scorer wrapper.
#[derive(Debug)]
pub struct ArtifactScorer<'a> {
    artifact: &'a ModelArtifact,
    reset_count: u64,
}

impl<'a> ArtifactScorer<'a> {
    /// Wrap an artifact core for teacher-forced S3 scoring.
    #[must_use]
    pub const fn new(artifact: &'a ModelArtifact) -> Self {
        Self {
            artifact,
            reset_count: 0,
        }
    }

    /// Borrow the wrapped artifact.
    #[must_use]
    pub const fn artifact(&self) -> &ModelArtifact {
        self.artifact
    }

    /// Number of reset notifications observed by this wrapper.
    #[must_use]
    pub const fn reset_count(&self) -> u64 {
        self.reset_count
    }

    /// Evaluate one per-token vocab row for the provided reset-chunk prefix.
    #[must_use]
    pub fn forward_logits(&self, prefix: &[CharId]) -> Vec<f32> {
        artifact_logits_from_core(self.artifact, prefix)
    }

    /// Reset provider-owned state between S3 chunks.
    pub fn reset_state(&mut self) {
        self.reset_count += 1;
    }
}

/// Deterministic fallback artifact row evaluator.
///
/// The artifact core currently carries canonical tensor metadata and
/// QuantSpec_S3 resolution facts, not in-repo tensor payload bytes. This helper
/// therefore derives a stable per-vocab row from deployable tensor hashes after
/// resolving each deployable weight through `QuantSpec_S3::weight_quant`.
#[must_use]
pub fn artifact_logits_from_core(artifact: &ModelArtifact, prefix: &[CharId]) -> Vec<f32> {
    let last = prefix.last().copied().unwrap_or(0);
    let mut logits = vec![0.0_f32; VOCAB_SIZE];

    for tensor in artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
    {
        let quant = artifact
            .core
            .quant
            .weight_quant(&tensor.id)
            .expect("ArtifactCore validates deployable weight QuantSpec coverage");
        for (index, logit) in logits.iter_mut().enumerate() {
            let digest = artifact_logit_digest(artifact, tensor, quant, last, index as u8);
            let centered = f32::from(digest.as_bytes()[index % 32]) / 255.0 - 0.5;
            *logit += centered * 0.25;
        }
    }

    logits
}

fn artifact_logit_digest(
    artifact: &ModelArtifact,
    tensor: &gbf_artifact::CanonicalTensor,
    quant: &WeightQuant,
    last: CharId,
    target: CharId,
) -> gbf_foundation::Hash256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"gbf-oracle:s3-artifact-scorer:v1\0");
    bytes.extend_from_slice(artifact.core.manifest.lineage.0.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(tensor.id.as_str().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(stable_dtype_name(tensor.dtype).as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(tensor.payload_sha.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(stable_quant_name(quant).as_bytes());
    bytes.push(0);
    bytes.push(last);
    bytes.push(target);
    sha256(bytes)
}

const fn stable_dtype_name(dtype: Dtype) -> &'static str {
    match dtype {
        Dtype::Fp32 => "fp32",
        Dtype::Ternary2 => "ternary2",
        Dtype::Q8_8 => "q8_8",
        Dtype::I32 => "i32",
    }
}

const fn stable_quant_name(quant: &WeightQuant) -> &'static str {
    match quant {
        WeightQuant::Fp32 => "fp32",
        WeightQuant::Ternary2 { .. } => "ternary2",
    }
}
