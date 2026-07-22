//! Import the recurrent state-checkpoint artifacts consumed by deployable ROMs.
//!
//! This is a compiler boundary, not benchmark infrastructure: every tensor is
//! resolved through the manifest and SHA-256 checked before model lowering.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use gbf_foundation::sha256;
use gbf_kernel::model_ref::TernaryLayer;
use gbf_kernel::state_model_ref::{
    BlockFfn, LogitPaging, LowRankRouter, StateCheckpoint, StateTopology,
};

/// Dense recurrent checkpoint schema emitted by the hardened-export bridge.
pub const DENSE_STATE_CHECKPOINT_SCHEMA: &str = "f_s5_state_checkpoint_export.v1";
/// Top-1 MoE recurrent checkpoint schema emitted by the S8 bridge.
pub const MOE_STATE_CHECKPOINT_SCHEMA: &str = "f_s8_moe_state_checkpoint_export.v2";

/// Loaded stateful checkpoint plus verified provenance facts.
#[derive(Debug)]
pub struct StateCheckpointBundle {
    pub checkpoint: StateCheckpoint,
    pub topology: StateTopology,
    pub manifest_schema: String,
    pub manifest_git_sha: String,
    pub manifest_sha256: String,
    pub tensors_verified: usize,
}

#[derive(Debug)]
pub enum StateCheckpointImportError {
    Io { path: PathBuf, reason: String },
    Manifest { reason: String },
    ShaMismatch { tensor: String },
    Model(String),
}

impl fmt::Display for StateCheckpointImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, reason } => write!(f, "io {}: {reason}", path.display()),
            Self::Manifest { reason } => write!(f, "manifest: {reason}"),
            Self::ShaMismatch { tensor } => write!(f, "sha256 mismatch for tensor {tensor}"),
            Self::Model(reason) => write!(f, "model: {reason}"),
        }
    }
}

impl Error for StateCheckpointImportError {}

/// Load and integrity-check a dense or MoE recurrent checkpoint export.
pub fn import_state_checkpoint(
    export_dir: &Path,
) -> Result<StateCheckpointBundle, StateCheckpointImportError> {
    let manifest_path = export_dir.join("manifest.json");
    let manifest_bytes =
        std::fs::read(&manifest_path).map_err(|e| StateCheckpointImportError::Io {
            path: manifest_path.clone(),
            reason: e.to_string(),
        })?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        StateCheckpointImportError::Manifest {
            reason: e.to_string(),
        }
    })?;
    let schema = manifest["schema"].as_str().unwrap_or_default().to_string();
    let is_moe_schema = match schema.as_str() {
        DENSE_STATE_CHECKPOINT_SCHEMA => false,
        MOE_STATE_CHECKPOINT_SCHEMA => true,
        _ => {
            return Err(StateCheckpointImportError::Manifest {
                reason: format!("unexpected schema {schema:?}"),
            });
        }
    };
    let git_sha = manifest["git_sha"].as_str().unwrap_or_default().to_string();

    let topo = &manifest["topology"];
    let dim = |v: &serde_json::Value, what: &str| -> Result<usize, StateCheckpointImportError> {
        v.as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| StateCheckpointImportError::Manifest {
                reason: format!("topology.{what} missing or non-integer"),
            })
    };
    let manifest_is_moe = topo["moe"].as_bool() == Some(true);
    if manifest_is_moe != is_moe_schema {
        return Err(StateCheckpointImportError::Manifest {
            reason: format!("schema {schema:?} and topology.moe {manifest_is_moe} disagree"),
        });
    }
    let n_experts = if is_moe_schema {
        dim(&topo["n_experts_per_block"], "n_experts_per_block")?
    } else {
        1
    };
    let vocab = dim(&topo["vocab"], "vocab")?;
    let logit_paging = if vocab > gbf_kernel::state_model_ref::LOGIT_PAGE_IDS {
        LogitPaging::Paged
    } else {
        LogitPaging::SinglePage
    };
    let topology = StateTopology {
        d_model: dim(&topo["d_model"], "d_model")?,
        d_ff: dim(&topo["d_ff"], "d_ff")?,
        n_blocks: dim(&topo["n_blocks"], "n_blocks")?,
        state_slots: dim(
            &topo["sequence_state_params"]["state_slots"],
            "sequence_state_params.state_slots",
        )?,
        vocab,
        n_experts,
        logit_paging,
    };

    let tensors =
        manifest["tensors"]
            .as_array()
            .ok_or_else(|| StateCheckpointImportError::Manifest {
                reason: "missing tensors array".into(),
            })?;
    let mut verified = 0usize;
    let mut load = |name: &str| -> Result<Vec<u8>, StateCheckpointImportError> {
        let entry = tensors
            .iter()
            .find(|tensor| tensor["name"].as_str() == Some(name))
            .ok_or_else(|| StateCheckpointImportError::Manifest {
                reason: format!("tensor {name} missing"),
            })?;
        let file = entry["file"]
            .as_str()
            .ok_or_else(|| StateCheckpointImportError::Manifest {
                reason: format!("tensor {name} missing file"),
            })?;
        let path = export_dir.join(file);
        let bytes = std::fs::read(&path).map_err(|e| StateCheckpointImportError::Io {
            path,
            reason: e.to_string(),
        })?;
        let expected = entry["sha256"].as_str().unwrap_or_default();
        if sha256(&bytes).to_hex() != expected {
            return Err(StateCheckpointImportError::ShaMismatch {
                tensor: name.to_string(),
            });
        }
        verified += 1;
        Ok(bytes)
    };

    let emb_bytes = load("embedding")?;
    if emb_bytes.len() != topology.vocab * topology.d_model * 4 {
        return Err(StateCheckpointImportError::Manifest {
            reason: format!("embedding byte length {}", emb_bytes.len()),
        });
    }
    let embedding: Vec<f32> = emb_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let decay_bytes = load("state_decay")?;
    let decay_raw: Vec<u16> = decay_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let decode_ternary = |tern: &[u8],
                          scales: &[u8],
                          rows: usize,
                          cols: usize|
     -> Result<TernaryLayer, StateCheckpointImportError> {
        let weights: Vec<i8> = tern.iter().map(|&b| b as i8).collect();
        let scales_raw: Vec<u16> = scales
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        TernaryLayer::new(rows, cols, weights, scales_raw)
            .map_err(|e| StateCheckpointImportError::Model(e.to_string()))
    };
    macro_rules! ternary {
        ($base:expr, $rows:expr, $cols:expr) => {{
            let base: String = $base;
            let tern = load(&format!("{base}.ternary"))?;
            let scales = load(&format!("{base}.scales"))?;
            decode_ternary(&tern, &scales, $rows, $cols)
        }};
    }

    let state_in = ternary!(
        "state_input_to_state".to_string(),
        topology.state_slots,
        topology.d_model
    )?;
    let state_out = ternary!(
        "state_state_to_output".to_string(),
        topology.d_model,
        topology.state_slots
    )?;

    let checkpoint = if is_moe_schema {
        let layers =
            manifest["layers"]
                .as_array()
                .ok_or_else(|| StateCheckpointImportError::Manifest {
                    reason: "MoE manifest missing layers array".into(),
                })?;
        macro_rules! load_f32 {
            ($name:expr) => {{
                let name: String = $name;
                let bytes = load(&name)?;
                if bytes.len() % 4 != 0 {
                    return Err(StateCheckpointImportError::Manifest {
                        reason: format!("router tensor {name} not f32-aligned"),
                    });
                }
                bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect::<Vec<f32>>()
            }};
        }
        let mut blocks = Vec::with_capacity(topology.n_blocks);
        for k in 0..topology.n_blocks {
            let router_rank = layers
                .get(k)
                .and_then(|layer| layer["router_rank"].as_u64())
                .map(|n| n as usize)
                .ok_or_else(|| StateCheckpointImportError::Manifest {
                    reason: format!("layers[{k}].router_rank missing"),
                })?;
            let router = LowRankRouter::new(
                router_rank,
                topology.d_model,
                topology.n_experts,
                load_f32!(format!("block{k}_router_input_projection")),
                load_f32!(format!("block{k}_router_input_bias")),
                load_f32!(format!("block{k}_router_expert_projection")),
                load_f32!(format!("block{k}_router_expert_bias")),
            )
            .map_err(|e| StateCheckpointImportError::Model(e.to_string()))?;
            let mut experts = Vec::with_capacity(topology.n_experts);
            for e in 0..topology.n_experts {
                experts.push((
                    ternary!(
                        format!("block{k}_expert{e}_up"),
                        topology.d_ff,
                        topology.d_model
                    )?,
                    ternary!(
                        format!("block{k}_expert{e}_down"),
                        topology.d_model,
                        topology.d_ff
                    )?,
                ));
            }
            blocks.push(BlockFfn::Moe { router, experts });
        }
        StateCheckpoint::new_moe(topology, embedding, state_in, state_out, decay_raw, blocks)
            .map_err(|e| StateCheckpointImportError::Model(e.to_string()))?
    } else {
        let mut blocks = Vec::with_capacity(topology.n_blocks);
        for k in 0..topology.n_blocks {
            blocks.push(gbf_kernel::model_ref::BlockWeights {
                up: ternary!(format!("block{k}_up"), topology.d_ff, topology.d_model)?,
                down: ternary!(format!("block{k}_down"), topology.d_model, topology.d_ff)?,
            });
        }
        StateCheckpoint::new(topology, embedding, state_in, state_out, decay_raw, blocks)
            .map_err(|e| StateCheckpointImportError::Model(e.to_string()))?
    };

    Ok(StateCheckpointBundle {
        checkpoint,
        topology,
        manifest_schema: schema,
        manifest_git_sha: git_sha,
        manifest_sha256: sha256(&manifest_bytes).to_hex(),
        tensors_verified: verified,
    })
}

/// Compatibility name for callers migrating from the former benchmark owner.
pub use import_state_checkpoint as load_state_checkpoint;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_committed_dense_state_checkpoint() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let export = root.join("experiments/S5/state-ab/checkpoint-export");
        let bundle = import_state_checkpoint(&export).expect("committed state export imports");
        assert_eq!(bundle.manifest_schema, DENSE_STATE_CHECKPOINT_SCHEMA);
        assert_eq!(bundle.topology.n_experts, 1);
        assert!(bundle.tensors_verified > 0);
    }
}
