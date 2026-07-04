//! Inference lowering: [`ArtifactCore`] + [`ExportTopology`] -> a narrow
//! typed model program for the dense-bigram model class (bd-1skgm).
//!
//! **Scope note (documented decision):** this is deliberately *not* the full
//! Stage 3 `GbInferIR` (`crate::s3::infer_ir`). `GbInferIR` construction
//! requires the whole Stage 0-2 product chain (quant-graph identity, static
//! budget report self-hashes, policy ingress projections, provenance/effect
//! totality) that no real producer emits for a checkpoint export today.
//! Wiring the export through that apparatus would mean fabricating those
//! upstream products. Instead this module defines [`DenseBigramProgram`], a
//! narrow IR that types exactly the op sequence the deployed model executes,
//! with every tensor reference resolved through the artifact `QuantSpec`
//! (never by tensor-id naming conventions). Migrating this model class onto
//! `GbInferIR` remains future work owned by the Stage 3 pipeline beads.
//!
//! Op sequence typed here (the pinned v0 numeric contract orders these):
//!
//! ```text
//! embed(prev_byte) -> x
//! for each block: norm_quant(x) -> up matvec -> gelu -> down matvec -> x += delta
//! norm_quant(x) -> tied head matvec -> argmax
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::norm_plan::{NormPlan, TileRmsThenAffineClipPlan};
use gbf_artifact::quant::{ActivationNonlinearitySpec, ActivationQuantEntry};
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorId, CanonicalTensorKind};
use gbf_artifact::weight_plan::{ScaleFormat, ScaleGranularity, WeightEncoding};

use crate::import_checkpoint_export::ExportTopology;

// ---------------------------------------------------------------------------
// program IR
// ---------------------------------------------------------------------------

/// Fully resolved ternary matvec op: projection path plus the weight/scale
/// tensor ids and dimensions, resolved through the ternary quant plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TernaryMatvecOp {
    pub projection: ArtifactPath,
    pub weight: CanonicalTensorId,
    pub scale: CanonicalTensorId,
    pub rows: usize,
    pub cols: usize,
}

/// Full-vector RMS norm + activation quantization op.
#[derive(Debug, Clone, PartialEq)]
pub struct NormQuantOp {
    pub norm: ArtifactPath,
    pub plan: TileRmsThenAffineClipPlan,
    /// Signed activation grid half-width (127 for Int8 symmetric).
    pub qmax: i32,
    /// Activation range half-width in real units.
    pub range: f32,
}

/// Elementwise GELU on the activation grid.
#[derive(Debug, Clone, PartialEq)]
pub struct GeluOp {
    pub activation: ArtifactPath,
    pub qmax: i32,
    pub range: f32,
}

/// One pre-norm residual FFN block in execution order.
#[derive(Debug, Clone, PartialEq)]
pub struct FfnBlockProgram {
    pub index: usize,
    pub norm: NormQuantOp,
    pub up: TernaryMatvecOp,
    pub gelu: GeluOp,
    pub down: TernaryMatvecOp,
}

/// The typed model program for the dense-bigram class.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseBigramProgram {
    pub d_model: usize,
    pub d_ff: usize,
    pub vocab: usize,
    /// Full-precision embedding tensor (resolved through `weight_quant`);
    /// also the tied head.
    pub embedding: CanonicalTensorId,
    pub blocks: Vec<FfnBlockProgram>,
    pub final_norm: NormQuantOp,
}

impl DenseBigramProgram {
    /// Names of the ops in execution order (used by kernel selection and the
    /// build report).
    #[must_use]
    pub fn op_names(&self) -> Vec<String> {
        let mut ops = vec!["embed_lookup".to_string()];
        for block in &self.blocks {
            let k = block.index;
            ops.push(format!("block{k}.norm_quant"));
            ops.push(format!("block{k}.up.matvec"));
            ops.push(format!("block{k}.up.scale_epilogue"));
            ops.push(format!("block{k}.gelu"));
            ops.push(format!("block{k}.down.matvec"));
            ops.push(format!("block{k}.down.scale_epilogue_residual_add"));
        }
        ops.push("final.norm_quant".to_string());
        ops.push("head.tied_matvec".to_string());
        ops.push("head.argmax".to_string());
        ops
    }
}

// ---------------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum LowerInferError {
    MissingTernaryEntry {
        projection: ArtifactPath,
    },
    UnsupportedTernaryPlan {
        projection: ArtifactPath,
        reason: &'static str,
    },
    MissingTensor {
        id: CanonicalTensorId,
    },
    MissingEmbeddingQuantEntry,
    EmbeddingKindMismatch {
        id: CanonicalTensorId,
    },
    ShapeChainBroken {
        what: String,
    },
    MissingNormPlan {
        norm: ArtifactPath,
    },
    UnsupportedNormPlan {
        norm: ArtifactPath,
        reason: &'static str,
    },
    MissingActivationEntry {
        activation: ArtifactPath,
    },
    UnsupportedActivationEntry {
        activation: ArtifactPath,
        reason: &'static str,
    },
}

impl fmt::Display for LowerInferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTernaryEntry { projection } => {
                write!(f, "no ternary quant entry for projection {projection}")
            }
            Self::UnsupportedTernaryPlan { projection, reason } => {
                write!(f, "ternary plan for {projection} unsupported: {reason}")
            }
            Self::MissingTensor { id } => write!(f, "tensor {id} missing from artifact core"),
            Self::MissingEmbeddingQuantEntry => f.write_str(
                "no full-precision weight_quant entry resolves an Embedding-kind tensor",
            ),
            Self::EmbeddingKindMismatch { id } => {
                write!(f, "topology embedding {id} is not an Embedding-kind tensor")
            }
            Self::ShapeChainBroken { what } => write!(f, "shape chain broken: {what}"),
            Self::MissingNormPlan { norm } => write!(f, "no norm plan for {norm}"),
            Self::UnsupportedNormPlan { norm, reason } => {
                write!(f, "norm plan for {norm} unsupported: {reason}")
            }
            Self::MissingActivationEntry { activation } => {
                write!(f, "no activation quant entry for {activation}")
            }
            Self::UnsupportedActivationEntry { activation, reason } => {
                write!(f, "activation entry {activation} unsupported: {reason}")
            }
        }
    }
}

impl Error for LowerInferError {}

// ---------------------------------------------------------------------------
// lowering
// ---------------------------------------------------------------------------

fn tensor_by_id(core: &ArtifactCore) -> BTreeMap<&CanonicalTensorId, &CanonicalTensor> {
    core.tensors().iter().map(|t| (&t.id, t)).collect()
}

fn resolve_matvec(
    core: &ArtifactCore,
    tensors: &BTreeMap<&CanonicalTensorId, &CanonicalTensor>,
    projection: &ArtifactPath,
) -> Result<TernaryMatvecOp, LowerInferError> {
    let entry = core
        .quant()
        .ternary_weight_plans()
        .iter()
        .find(|entry| &entry.projection == projection)
        .ok_or_else(|| LowerInferError::MissingTernaryEntry {
            projection: projection.clone(),
        })?;
    if entry.plan.encoding != WeightEncoding::Ternary2 {
        return Err(LowerInferError::UnsupportedTernaryPlan {
            projection: projection.clone(),
            reason: "only Ternary2 encoding is lowerable",
        });
    }
    if entry.plan.scale_granularity != ScaleGranularity::PerOutputRow {
        return Err(LowerInferError::UnsupportedTernaryPlan {
            projection: projection.clone(),
            reason: "only per-output-row scales are lowerable",
        });
    }
    if entry.plan.scale_format != ScaleFormat::Q8_8 {
        return Err(LowerInferError::UnsupportedTernaryPlan {
            projection: projection.clone(),
            reason: "only Q8.8 scales are lowerable",
        });
    }
    let weight = tensors
        .get(&entry.weight)
        .ok_or_else(|| LowerInferError::MissingTensor {
            id: entry.weight.clone(),
        })?;
    let dims = weight.layout.shape.dims();
    Ok(TernaryMatvecOp {
        projection: projection.clone(),
        weight: entry.weight.clone(),
        scale: entry.scale.clone(),
        rows: dims[0] as usize,
        cols: dims[1] as usize,
    })
}

fn resolve_norm(core: &ArtifactCore, norm: &ArtifactPath) -> Result<NormQuantOp, LowerInferError> {
    let entry = core
        .quant()
        .norm_plans()
        .iter()
        .find(|entry| &entry.norm == norm)
        .ok_or_else(|| LowerInferError::MissingNormPlan { norm: norm.clone() })?;
    let NormPlan::TileRmsThenAffineClip(plan) = &entry.plan else {
        return Err(LowerInferError::UnsupportedNormPlan {
            norm: norm.clone(),
            reason: "only tile-RMS-then-affine-clip norms are lowerable",
        });
    };
    Ok(NormQuantOp {
        norm: norm.clone(),
        plan: *plan,
        qmax: 127,
        range: plan.clip.hi,
    })
}

fn find_activation(
    core: &ArtifactCore,
    nonlinearity: ActivationNonlinearitySpec,
) -> Option<&ActivationQuantEntry> {
    core.quant()
        .activation_quant()
        .iter()
        .find(|entry| entry.nonlinearity == nonlinearity)
}

/// Lower the artifact core plus the export topology into the typed model
/// program. Every reference resolves through the quant spec; shape chains
/// are checked end to end.
pub fn lower_infer(
    core: &ArtifactCore,
    topology: &ExportTopology,
) -> Result<DenseBigramProgram, LowerInferError> {
    let tensors = tensor_by_id(core);

    // Embedding: resolve through weight_quant (full-precision entry whose
    // tensor is Embedding-kind), then confirm it is the topology's embedding.
    let embedding_entry = core
        .quant()
        .weight_quant()
        .iter()
        .filter(|entry| entry.ternary_plan.is_none())
        .find(|entry| {
            tensors
                .get(&entry.tensor)
                .is_some_and(|t| t.kind == CanonicalTensorKind::Embedding)
        })
        .ok_or(LowerInferError::MissingEmbeddingQuantEntry)?;
    if embedding_entry.tensor != topology.embedding {
        return Err(LowerInferError::EmbeddingKindMismatch {
            id: topology.embedding.clone(),
        });
    }
    let embedding =
        tensors
            .get(&embedding_entry.tensor)
            .ok_or_else(|| LowerInferError::MissingTensor {
                id: embedding_entry.tensor.clone(),
            })?;
    let emb_dims = embedding.layout.shape.dims();
    if emb_dims.len() != 2
        || emb_dims[0] as usize != topology.vocab
        || emb_dims[1] as usize != topology.d_model
    {
        return Err(LowerInferError::ShapeChainBroken {
            what: format!(
                "embedding shape {emb_dims:?} vs topology [{}, {}]",
                topology.vocab, topology.d_model
            ),
        });
    }

    // Activation grids: norm-quant (Identity) and FFN GELU (GeluClip).
    let norm_act =
        find_activation(core, ActivationNonlinearitySpec::Identity).ok_or_else(|| {
            LowerInferError::MissingActivationEntry {
                activation: ArtifactPath::new("activation.norm_quant")
                    .expect("static path is valid"),
            }
        })?;
    let gelu_act =
        find_activation(core, ActivationNonlinearitySpec::GeluClip).ok_or_else(|| {
            LowerInferError::MissingActivationEntry {
                activation: ArtifactPath::new("activation.ffn_gelu").expect("static path is valid"),
            }
        })?;
    for entry in [norm_act, gelu_act] {
        if entry.range.lo != -entry.range.hi {
            return Err(LowerInferError::UnsupportedActivationEntry {
                activation: entry.activation.clone(),
                reason: "activation range must be symmetric",
            });
        }
    }

    let mut blocks = Vec::with_capacity(topology.blocks.len());
    for block in &topology.blocks {
        let up = resolve_matvec(core, &tensors, &block.up_projection)?;
        let down = resolve_matvec(core, &tensors, &block.down_projection)?;
        if up.cols != topology.d_model || up.rows != topology.d_ff {
            return Err(LowerInferError::ShapeChainBroken {
                what: format!(
                    "block {} up {}x{} vs expected {}x{}",
                    block.index, up.rows, up.cols, topology.d_ff, topology.d_model
                ),
            });
        }
        if down.cols != topology.d_ff || down.rows != topology.d_model {
            return Err(LowerInferError::ShapeChainBroken {
                what: format!(
                    "block {} down {}x{} vs expected {}x{}",
                    block.index, down.rows, down.cols, topology.d_model, topology.d_ff
                ),
            });
        }
        let norm_path = ArtifactPath::new(format!("block{}.norm", block.index)).map_err(|_| {
            LowerInferError::MissingNormPlan {
                norm: block.up_projection.clone(),
            }
        })?;
        blocks.push(FfnBlockProgram {
            index: block.index,
            norm: resolve_norm(core, &norm_path)?,
            gelu: GeluOp {
                activation: gelu_act.activation.clone(),
                qmax: 127,
                range: gelu_act.range.hi,
            },
            up,
            down,
        });
    }

    let final_norm_path = ArtifactPath::new("final.norm").expect("static path is valid");
    Ok(DenseBigramProgram {
        d_model: topology.d_model,
        d_ff: topology.d_ff,
        vocab: topology.vocab,
        embedding: embedding_entry.tensor.clone(),
        blocks,
        final_norm: resolve_norm(core, &final_norm_path)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_checkpoint_export::{
        import_checkpoint_export, write_synthetic_checkpoint_export,
    };

    fn program(seed: u64) -> DenseBigramProgram {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), seed).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        lower_infer(&imported.core, &imported.topology).expect("lowers")
    }

    #[test]
    fn lower_infer_types_the_full_op_sequence() {
        let program = program(9);
        assert_eq!(program.blocks.len(), 4);
        assert_eq!(program.d_model, 64);
        assert_eq!(program.d_ff, 128);
        assert_eq!(program.vocab, 256);
        assert_eq!(program.embedding.as_str(), "embedding");
        for (k, block) in program.blocks.iter().enumerate() {
            assert_eq!(block.index, k);
            assert_eq!((block.up.rows, block.up.cols), (128, 64));
            assert_eq!((block.down.rows, block.down.cols), (64, 128));
            assert_eq!(block.norm.qmax, 127);
            assert_eq!(block.norm.range, 8.0);
            assert_eq!(block.gelu.range, 8.0);
        }
        // embed + 4 blocks x 6 ops + final norm + head + argmax.
        assert_eq!(program.op_names().len(), 1 + 4 * 6 + 3);
    }

    #[test]
    fn lower_infer_rejects_missing_ternary_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 9).expect("writes export");
        let imported = import_checkpoint_export(dir.path()).expect("imports");
        let mut topology = imported.topology.clone();
        topology.blocks[0].up_projection =
            ArtifactPath::new("no_such_projection").expect("valid path");
        let err = lower_infer(&imported.core, &topology).expect_err("must reject");
        assert!(matches!(err, LowerInferError::MissingTernaryEntry { .. }));
    }
}
