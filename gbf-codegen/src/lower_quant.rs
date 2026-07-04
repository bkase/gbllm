//! Quantization lowering: typed model program + artifact payloads -> the
//! canonical integer-lowered model (bd-1skgm).
//!
//! Pulls the tensor payloads referenced by a [`DenseBigramProgram`] out of
//! the [`ArtifactCore`] and lowers them through the pinned v0 numeric
//! contract (`history/planv0.md` "Session amendment 2026-07-04" §3) via
//! [`gbf_kernel::model_ref`]: Q11.5 embedding rows, per-tensor i8 tied head,
//! 255-entry GELU LUT, per-row `-128 * sum(row)` accumulator seeds. The
//! integer semantics themselves live in `gbf-kernel` (the eDSL/builder home);
//! this stage owns resolving artifact payloads into that contract.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorId};
use gbf_kernel::model_ref::{
    BlockWeights, DenseBigramCheckpoint, IntLoweredModel, ModelRefError, TernaryLayer,
};

use crate::lower_infer::{DenseBigramProgram, TernaryMatvecOp};

/// Quant-lowered model: the reconstructed checkpoint plus the canonical
/// integer lowering, with reporting facts observed during lowering.
#[derive(Debug, Clone)]
pub struct QuantLoweredModel {
    pub checkpoint: DenseBigramCheckpoint,
    pub model: IntLoweredModel,
    /// Fraction of zero ternary weights in permille (reporting only).
    pub weight_zero_permille: u32,
    /// Maximum raw Q8.8 row scale over all projections.
    pub max_scale_raw: u16,
    /// Maximum |embedding| value (drives the Q11.5 / head-i8 quantization).
    pub max_abs_embedding: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LowerQuantError {
    MissingTensor { id: CanonicalTensorId },
    WrongPayload { id: CanonicalTensorId },
    Model(ModelRefError),
}

impl fmt::Display for LowerQuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTensor { id } => write!(f, "tensor {id} missing from artifact core"),
            Self::WrongPayload { id } => {
                write!(f, "tensor {id} payload type disagrees with its quant plan")
            }
            Self::Model(error) => write!(f, "numeric-contract lowering: {error}"),
        }
    }
}

impl Error for LowerQuantError {}

impl From<ModelRefError> for LowerQuantError {
    fn from(error: ModelRefError) -> Self {
        Self::Model(error)
    }
}

fn require<'a>(
    tensors: &BTreeMap<&CanonicalTensorId, &'a CanonicalTensor>,
    id: &CanonicalTensorId,
) -> Result<&'a CanonicalTensor, LowerQuantError> {
    tensors
        .get(id)
        .copied()
        .ok_or_else(|| LowerQuantError::MissingTensor { id: id.clone() })
}

fn ternary_layer(
    tensors: &BTreeMap<&CanonicalTensorId, &CanonicalTensor>,
    op: &TernaryMatvecOp,
) -> Result<TernaryLayer, LowerQuantError> {
    let weight = require(tensors, &op.weight)?;
    let weights = weight
        .payload
        .as_i8_slice()
        .ok_or_else(|| LowerQuantError::WrongPayload {
            id: op.weight.clone(),
        })?
        .to_vec();
    let scale = require(tensors, &op.scale)?;
    let scales = scale
        .payload
        .as_u16_slice()
        .ok_or_else(|| LowerQuantError::WrongPayload {
            id: op.scale.clone(),
        })?
        .to_vec();
    Ok(TernaryLayer::new(op.rows, op.cols, weights, scales)?)
}

/// Lower the program's payloads into the canonical integer model.
pub fn lower_quant(
    core: &ArtifactCore,
    program: &DenseBigramProgram,
) -> Result<QuantLoweredModel, LowerQuantError> {
    let tensors: BTreeMap<&CanonicalTensorId, &CanonicalTensor> =
        core.tensors().iter().map(|t| (&t.id, t)).collect();

    let embedding_tensor = require(&tensors, &program.embedding)?;
    let embedding = embedding_tensor
        .payload
        .as_f32_slice()
        .ok_or_else(|| LowerQuantError::WrongPayload {
            id: program.embedding.clone(),
        })?
        .to_vec();
    let max_abs_embedding = embedding.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));

    let mut blocks = Vec::with_capacity(program.blocks.len());
    let mut zeros = 0usize;
    let mut total = 0usize;
    let mut max_scale_raw = 0u16;
    for block in &program.blocks {
        let up = ternary_layer(&tensors, &block.up)?;
        let down = ternary_layer(&tensors, &block.down)?;
        for layer in [&up, &down] {
            let elems = layer.rows() * layer.cols();
            zeros += layer.zero_permille() as usize * elems / 1000;
            total += elems;
            for row in 0..layer.rows() {
                max_scale_raw = max_scale_raw.max(layer.scale_raw(row));
            }
        }
        blocks.push(BlockWeights { up, down });
    }
    let weight_zero_permille = (zeros * 1000 / total.max(1)) as u32;

    let checkpoint = DenseBigramCheckpoint::new(embedding, blocks)?;
    let model = IntLoweredModel::lower(&checkpoint)?;

    Ok(QuantLoweredModel {
        checkpoint,
        model,
        weight_zero_permille,
        max_scale_raw,
        max_abs_embedding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_checkpoint_export::{
        import_checkpoint_export, write_synthetic_checkpoint_export,
    };
    use crate::lower_infer::lower_infer;
    use gbf_kernel::model_ref::synthetic_checkpoint;

    /// The artifact round trip (synthetic checkpoint -> export files ->
    /// import -> lower) must reproduce the direct in-memory lowering exactly:
    /// same forward-pass logits and argmax for probe bytes.
    #[test]
    fn lower_quant_round_trips_the_synthetic_checkpoint_exactly() {
        let seed = 11;
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), seed).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        let program = lower_infer(&imported.core, &imported.topology).expect("lowers infer");
        let lowered = lower_quant(&imported.core, &program).expect("lowers quant");

        let direct =
            IntLoweredModel::lower(&synthetic_checkpoint(seed)).expect("direct lowering works");
        for probe in [0x00u8, 0x0A, 0x41, 0x7F, 0xFF] {
            let via_artifact = lowered.model.forward(probe);
            let direct_trace = direct.forward(probe);
            assert_eq!(
                via_artifact.logits, direct_trace.logits,
                "probe {probe:#04x}"
            );
            assert_eq!(
                via_artifact.argmax, direct_trace.argmax,
                "probe {probe:#04x}"
            );
        }
        assert!(lowered.max_scale_raw > 0);
        assert!(lowered.max_abs_embedding > 0.0);
    }
}
