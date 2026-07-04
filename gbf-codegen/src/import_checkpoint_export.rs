//! Checkpoint-export importer: `f_s6_dense_checkpoint_export.v1` bundle ->
//! validated [`gbf_artifact::core::ArtifactCore`] (bd-1skgm).
//!
//! This is the compiler's real front door for the trained dense-bigram
//! checkpoint (`experiments/S6/checkpoint-export`): it parses the manifest,
//! sha256-verifies every tensor file against the committed manifest digests,
//! decodes the payloads (f32 LE embedding, `{-1,0,+1}` i8 ternary weights,
//! u16 LE raw Q8.8 per-output-row scales), and constructs a validated
//! `ArtifactCore` (canonical tensors + `QuantSpec` + sequence semantics).
//!
//! Alongside the core, the importer returns a typed [`ExportTopology`] — the
//! ordered block structure the manifest declares. `ArtifactCore` is a bag of
//! tensors plus quant metadata with no graph, so block order is program
//! structure that must be carried explicitly rather than re-derived from
//! tensor-id naming conventions downstream.
//!
//! Sequence semantics: the export declares `stateless_bigram_context`, which
//! has no artifact-schema variant today. The importer records the minimal
//! `SequenceSemanticsSpec::linear_state` placeholder (one 4-byte slot) and
//! names it as a placeholder in [`CheckpointExportFacts`].

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use gbf_artifact::core::{ArtifactCore, ArtifactCoreError};
use gbf_artifact::ids::{ArtifactPath, ArtifactPathError};
use gbf_artifact::norm_plan::{NormAffineParams, NormClipBounds, NormPlan, NormTileRmsSpec};
use gbf_artifact::quant::{
    ActivationEvalModeSpec, ActivationNonlinearitySpec, ActivationQuantEntry,
    ActivationQuantFormatSpec, ActivationRangeModeSpec, ActivationRangeSpec, NormQuantEntry,
    QuantSpec, TernaryQuantEntry, WeightQuantEntry,
};
use gbf_artifact::sequence::{LINEAR_STATE_SLOT_BYTES, SequenceSemanticsSpec};
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorError, CanonicalTensorId, CanonicalTensorKind,
    CanonicalTensorLayout, CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
};
use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_foundation::sha256;
use serde::Deserialize;

/// The only export schema this importer accepts.
pub const CHECKPOINT_EXPORT_SCHEMA: &str = "f_s6_dense_checkpoint_export.v1";

// ---------------------------------------------------------------------------
// manifest shapes (deserialized, unknown fields tolerated)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    #[serde(default)]
    git_sha: String,
    layers: Vec<ManifestLayer>,
    numeric_convention: NumericConvention,
    tensors: Vec<ManifestTensor>,
    topology: ManifestTopology,
}

#[derive(Debug, Deserialize)]
struct ManifestLayer {
    index: u32,
    kind: String,
    up_ternary: String,
    up_scales: String,
    up_shape: [u32; 2],
    down_ternary: String,
    down_scales: String,
    down_shape: [u32; 2],
}

#[derive(Debug, Deserialize)]
struct ManifestTensor {
    name: String,
    file: String,
    sha256: String,
    shape: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct ManifestTopology {
    d_ff: u32,
    d_model: u32,
    family: String,
    moe: bool,
    n_blocks: u32,
    sequence_state_kind: String,
    tied_head: bool,
    vocab: u32,
}

#[derive(Debug, Deserialize)]
struct NumericConvention {
    activation_fake_quant: ActivationFakeQuant,
    norm: NormConvention,
    weight_encoding: String,
    weight_scale: String,
    embedding_dtype: String,
}

#[derive(Debug, Deserialize)]
struct ActivationFakeQuant {
    format: String,
    quant_steps: u32,
    range_hi: f32,
    range_lo: f32,
}

#[derive(Debug, Deserialize)]
struct NormConvention {
    affine_bias: f32,
    affine_scale: f32,
    clip_hi: f32,
    clip_lo: f32,
    epsilon: f32,
    kind: String,
}

// ---------------------------------------------------------------------------
// public result
// ---------------------------------------------------------------------------

/// Ordered program structure carried alongside the [`ArtifactCore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportTopology {
    /// Artifact path of the full-precision embedding (also the tied head).
    pub embedding: ArtifactPath,
    /// Blocks in execution order.
    pub blocks: Vec<ExportBlockRefs>,
    pub d_model: usize,
    pub d_ff: usize,
    pub vocab: usize,
}

/// Projection paths for one pre-norm residual FFN block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportBlockRefs {
    pub index: usize,
    pub up_projection: ArtifactPath,
    pub down_projection: ArtifactPath,
}

/// Provenance facts recorded by the importer for the build report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CheckpointExportFacts {
    pub schema: String,
    pub trainer_git_sha: String,
    /// sha256 of the manifest file itself; the manifest pins every tensor's
    /// sha256, all of which are verified on load.
    pub manifest_sha256: String,
    pub tensors_verified_sha256: usize,
    /// The export's declared sequence-state kind and the placeholder the
    /// artifact schema records for it (no stateless variant exists today).
    pub sequence_state_kind: String,
    pub sequence_semantics_placeholder: String,
}

/// Importer output: validated artifact core plus typed topology + facts.
#[derive(Debug, Clone)]
pub struct ImportedCheckpointExport {
    pub core: ArtifactCore,
    pub topology: ExportTopology,
    pub facts: CheckpointExportFacts,
}

// ---------------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CheckpointImportError {
    Io {
        path: PathBuf,
        reason: String,
    },
    Manifest {
        reason: String,
    },
    UnsupportedSchema {
        schema: String,
    },
    UnsupportedConvention {
        field: &'static str,
        observed: String,
    },
    ShaMismatch {
        tensor: String,
        expected: String,
        computed: String,
    },
    TensorDecode {
        tensor: String,
        reason: String,
    },
    Path(ArtifactPathError),
    Tensor(CanonicalTensorError),
    Core(ArtifactCoreError),
}

impl fmt::Display for CheckpointImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, reason } => write!(f, "io {}: {reason}", path.display()),
            Self::Manifest { reason } => write!(f, "manifest: {reason}"),
            Self::UnsupportedSchema { schema } => {
                write!(
                    f,
                    "unsupported export schema {schema:?} (expected {CHECKPOINT_EXPORT_SCHEMA:?})"
                )
            }
            Self::UnsupportedConvention { field, observed } => {
                write!(f, "unsupported numeric convention {field}: {observed:?}")
            }
            Self::ShaMismatch {
                tensor,
                expected,
                computed,
            } => write!(
                f,
                "sha256 mismatch for tensor {tensor}: manifest {expected}, file {computed}"
            ),
            Self::TensorDecode { tensor, reason } => {
                write!(f, "tensor {tensor} decode failed: {reason}")
            }
            Self::Path(error) => write!(f, "artifact path: {error}"),
            Self::Tensor(error) => write!(f, "canonical tensor: {error}"),
            Self::Core(error) => write!(f, "artifact core: {error}"),
        }
    }
}

impl Error for CheckpointImportError {}

impl From<ArtifactPathError> for CheckpointImportError {
    fn from(error: ArtifactPathError) -> Self {
        Self::Path(error)
    }
}

impl From<CanonicalTensorError> for CheckpointImportError {
    fn from(error: CanonicalTensorError) -> Self {
        Self::Tensor(error)
    }
}

impl From<ArtifactCoreError> for CheckpointImportError {
    fn from(error: ArtifactCoreError) -> Self {
        Self::Core(error)
    }
}

// ---------------------------------------------------------------------------
// convention pins
// ---------------------------------------------------------------------------

/// Exact numeric-convention strings/values the downstream lowering assumes.
/// Anything else must be rejected here rather than silently miscompiled.
fn check_conventions(manifest: &Manifest) -> Result<(), CheckpointImportError> {
    let unsupported = |field: &'static str, observed: String| {
        Err(CheckpointImportError::UnsupportedConvention { field, observed })
    };

    let topology = &manifest.topology;
    if topology.family != "dense_ffn_bigram_context" {
        return unsupported("topology.family", topology.family.clone());
    }
    if topology.moe {
        return unsupported("topology.moe", "true".to_string());
    }
    if !topology.tied_head {
        return unsupported("topology.tied_head", "false".to_string());
    }
    if topology.sequence_state_kind != "stateless_bigram_context" {
        return unsupported(
            "topology.sequence_state_kind",
            topology.sequence_state_kind.clone(),
        );
    }

    let act = &manifest.numeric_convention.activation_fake_quant;
    if act.format != "Int8_symmetric" {
        return unsupported("activation_fake_quant.format", act.format.clone());
    }
    if act.quant_steps != 127 {
        return unsupported(
            "activation_fake_quant.quant_steps",
            act.quant_steps.to_string(),
        );
    }
    if act.range_lo != -8.0 || act.range_hi != 8.0 {
        return unsupported(
            "activation_fake_quant.range",
            format!("[{}, {}]", act.range_lo, act.range_hi),
        );
    }

    let norm = &manifest.numeric_convention.norm;
    if norm.kind != "tile_rms_then_affine_clip(full_vector)" {
        return unsupported("norm.kind", norm.kind.clone());
    }

    let nc = &manifest.numeric_convention;
    if nc.weight_encoding != "Ternary2 {-1,0,+1}" {
        return unsupported("weight_encoding", nc.weight_encoding.clone());
    }
    if nc.weight_scale != "per_output_row Q8.8 (u16 raw, f32 = raw/256)" {
        return unsupported("weight_scale", nc.weight_scale.clone());
    }
    if nc.embedding_dtype != "f32_le" {
        return unsupported("embedding_dtype", nc.embedding_dtype.clone());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// import
// ---------------------------------------------------------------------------

/// Deploy-side ternary plan for this export. The manifest does not record the
/// training-time threshold schedule (the weights arrive already ternarized),
/// so the plan pins the deployment-relevant facts — `Ternary2` encoding,
/// per-output-row Q8.8 scales — with the fixed-threshold tag; no downstream
/// consumer reads the threshold from a checkpoint export.
fn export_ternary_plan() -> TernaryWeightPlan {
    TernaryWeightPlan::new(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
        ThresholdPlan::FixedQ8_8,
    )
}

fn read_file(path: &Path) -> Result<Vec<u8>, CheckpointImportError> {
    std::fs::read(path).map_err(|e| CheckpointImportError::Io {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

struct VerifiedTensor {
    bytes: Vec<u8>,
    shape: Vec<u32>,
}

/// Load, sha-verify, and index the manifest's tensor files by name.
fn load_verified_tensors(
    export_dir: &Path,
    manifest: &Manifest,
) -> Result<BTreeMap<String, VerifiedTensor>, CheckpointImportError> {
    let mut by_name = BTreeMap::new();
    for entry in &manifest.tensors {
        let bytes = read_file(&export_dir.join(&entry.file))?;
        let computed = sha256(&bytes).to_hex();
        if computed != entry.sha256 {
            return Err(CheckpointImportError::ShaMismatch {
                tensor: entry.name.clone(),
                expected: entry.sha256.clone(),
                computed,
            });
        }
        if by_name
            .insert(
                entry.name.clone(),
                VerifiedTensor {
                    bytes,
                    shape: entry.shape.clone(),
                },
            )
            .is_some()
        {
            return Err(CheckpointImportError::Manifest {
                reason: format!("duplicate tensor name {}", entry.name),
            });
        }
    }
    Ok(by_name)
}

fn take_tensor<'a>(
    tensors: &'a BTreeMap<String, VerifiedTensor>,
    name: &str,
) -> Result<&'a VerifiedTensor, CheckpointImportError> {
    tensors
        .get(name)
        .ok_or_else(|| CheckpointImportError::Manifest {
            reason: format!("tensor {name} referenced by layers but missing from tensors"),
        })
}

fn decode_f32(name: &str, tensor: &VerifiedTensor) -> Result<Vec<f32>, CheckpointImportError> {
    if !tensor.bytes.len().is_multiple_of(4) {
        return Err(CheckpointImportError::TensorDecode {
            tensor: name.to_string(),
            reason: format!("byte length {} is not a multiple of 4", tensor.bytes.len()),
        });
    }
    Ok(tensor
        .bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn decode_i8(tensor: &VerifiedTensor) -> Vec<i8> {
    tensor.bytes.iter().map(|&b| b as i8).collect()
}

fn decode_u16(name: &str, tensor: &VerifiedTensor) -> Result<Vec<u16>, CheckpointImportError> {
    if !tensor.bytes.len().is_multiple_of(2) {
        return Err(CheckpointImportError::TensorDecode {
            tensor: name.to_string(),
            reason: format!("byte length {} is not a multiple of 2", tensor.bytes.len()),
        });
    }
    Ok(tensor
        .bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect())
}

fn canonical_shape(dims: &[u32]) -> Result<CanonicalTensorShape, CheckpointImportError> {
    CanonicalTensorShape::new(dims.to_vec()).map_err(CheckpointImportError::from)
}

/// Import a `f_s6_dense_checkpoint_export.v1` bundle into a validated
/// [`ArtifactCore`] plus its typed topology and provenance facts.
pub fn import_checkpoint_export(
    export_dir: &Path,
) -> Result<ImportedCheckpointExport, CheckpointImportError> {
    let manifest_path = export_dir.join("manifest.json");
    let manifest_bytes = read_file(&manifest_path)?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| CheckpointImportError::Manifest {
            reason: e.to_string(),
        })?;
    if manifest.schema != CHECKPOINT_EXPORT_SCHEMA {
        return Err(CheckpointImportError::UnsupportedSchema {
            schema: manifest.schema,
        });
    }
    check_conventions(&manifest)?;

    let d_model = manifest.topology.d_model;
    let d_ff = manifest.topology.d_ff;
    let vocab = manifest.topology.vocab;
    let n_blocks = manifest.topology.n_blocks as usize;
    if manifest.layers.len() != n_blocks {
        return Err(CheckpointImportError::Manifest {
            reason: format!(
                "topology declares {n_blocks} blocks but manifest lists {} layers",
                manifest.layers.len()
            ),
        });
    }

    let verified = load_verified_tensors(export_dir, &manifest)?;
    let tensors_verified = verified.len();

    let mut tensors: Vec<CanonicalTensor> = Vec::new();
    let mut ternary_entries: Vec<TernaryQuantEntry> = Vec::new();
    let mut weight_entries: Vec<WeightQuantEntry> = Vec::new();
    let mut norm_entries: Vec<NormQuantEntry> = Vec::new();
    let mut block_refs: Vec<ExportBlockRefs> = Vec::new();

    // Embedding (also the tied head).
    let embedding_id = CanonicalTensorId::new("embedding")?;
    let emb = take_tensor(&verified, "embedding")?;
    if emb.shape != [vocab, d_model] {
        return Err(CheckpointImportError::Manifest {
            reason: format!(
                "embedding shape {:?} disagrees with topology [{vocab}, {d_model}]",
                emb.shape
            ),
        });
    }
    tensors.push(CanonicalTensor::new(
        embedding_id.clone(),
        CanonicalTensorKind::Embedding,
        CanonicalTensorLayout::new(canonical_shape(&emb.shape)?, TensorElementType::Float32),
        CanonicalTensorPayload::F32(decode_f32("embedding", emb)?),
    )?);
    weight_entries.push(WeightQuantEntry::full_precision(
        embedding_id.clone(),
        embedding_id.clone(),
    ));

    // Blocks, in manifest order; indices must be contiguous from zero.
    let norm = &manifest.numeric_convention.norm;
    let norm_plan = NormPlan::tile_rms_then_affine_clip(
        NormTileRmsSpec {
            tile_width: u16::try_from(d_model).map_err(|_| CheckpointImportError::Manifest {
                reason: format!("d_model {d_model} exceeds u16"),
            })?,
            epsilon: norm.epsilon,
        },
        NormAffineParams {
            scale: norm.affine_scale,
            bias: norm.affine_bias,
        },
        NormClipBounds {
            lo: norm.clip_lo,
            hi: norm.clip_hi,
        },
    );

    for (position, layer) in manifest.layers.iter().enumerate() {
        if layer.index as usize != position {
            return Err(CheckpointImportError::Manifest {
                reason: format!(
                    "layer at position {position} declares index {} (must be contiguous)",
                    layer.index
                ),
            });
        }
        if layer.kind != "prenorm_residual_ffn" {
            return Err(CheckpointImportError::UnsupportedConvention {
                field: "layer.kind",
                observed: layer.kind.clone(),
            });
        }
        if layer.up_shape != [d_ff, d_model] || layer.down_shape != [d_model, d_ff] {
            return Err(CheckpointImportError::Manifest {
                reason: format!(
                    "layer {position} shapes up {:?} / down {:?} disagree with topology",
                    layer.up_shape, layer.down_shape
                ),
            });
        }

        let mut projection_paths = Vec::with_capacity(2);
        for (weight_name, scale_name, shape) in [
            (&layer.up_ternary, &layer.up_scales, layer.up_shape),
            (&layer.down_ternary, &layer.down_scales, layer.down_shape),
        ] {
            let weight_id = CanonicalTensorId::new(weight_name.clone())?;
            let scale_id = CanonicalTensorId::new(scale_name.clone())?;
            // Projection path: the shared prefix before the trailing
            // `.ternary` role segment (e.g. `block0_up`), validated against
            // the manifest's paired scale name.
            let projection_str = weight_name.strip_suffix(".ternary").ok_or_else(|| {
                CheckpointImportError::Manifest {
                    reason: format!("ternary tensor {weight_name} lacks a .ternary suffix"),
                }
            })?;
            if scale_name.strip_suffix(".scales") != Some(projection_str) {
                return Err(CheckpointImportError::Manifest {
                    reason: format!(
                        "scale tensor {scale_name} does not pair with weight {weight_name}"
                    ),
                });
            }
            let projection = ArtifactPath::new(projection_str)?;

            let weight = take_tensor(&verified, weight_name)?;
            if weight.shape != shape {
                return Err(CheckpointImportError::Manifest {
                    reason: format!(
                        "tensor {weight_name} shape {:?} disagrees with layer shape {shape:?}",
                        weight.shape
                    ),
                });
            }
            tensors.push(CanonicalTensor::new(
                weight_id.clone(),
                CanonicalTensorKind::TernaryWeight,
                CanonicalTensorLayout::new(
                    canonical_shape(&weight.shape)?,
                    TensorElementType::TernaryI2,
                ),
                CanonicalTensorPayload::I8(decode_i8(weight)),
            )?);

            let scale = take_tensor(&verified, scale_name)?;
            tensors.push(CanonicalTensor::new(
                scale_id.clone(),
                CanonicalTensorKind::TernaryScale,
                CanonicalTensorLayout::new(canonical_shape(&scale.shape)?, TensorElementType::Q8_8),
                CanonicalTensorPayload::U16(decode_u16(scale_name, scale)?),
            )?);

            let entry = TernaryQuantEntry {
                projection: projection.clone(),
                weight: weight_id,
                scale: scale_id,
                bias: None,
                plan: export_ternary_plan(),
            };
            weight_entries.push(WeightQuantEntry::ternary(
                entry.projection.clone(),
                entry.weight.clone(),
                entry.plan,
            ));
            ternary_entries.push(entry);
            projection_paths.push(projection);
        }

        let down_projection = projection_paths.pop().expect("two projections pushed");
        let up_projection = projection_paths.pop().expect("two projections pushed");
        block_refs.push(ExportBlockRefs {
            index: position,
            up_projection,
            down_projection,
        });
        norm_entries.push(NormQuantEntry {
            norm: ArtifactPath::new(format!("block{position}.norm"))?,
            plan: norm_plan.clone(),
            lut: None,
        });
    }

    norm_entries.push(NormQuantEntry {
        norm: ArtifactPath::new("final.norm")?,
        plan: norm_plan.clone(),
        lut: None,
    });

    let act = &manifest.numeric_convention.activation_fake_quant;
    let act_range = ActivationRangeSpec {
        lo: act.range_lo,
        hi: act.range_hi,
        mode: ActivationRangeModeSpec::Fixed,
    };
    let activation_entries = vec![
        ActivationQuantEntry {
            activation: ArtifactPath::new("activation.norm_quant")?,
            range: act_range,
            quant_format: ActivationQuantFormatSpec::Int8,
            eval_mode: ActivationEvalModeSpec::Quantized,
            nonlinearity: ActivationNonlinearitySpec::Identity,
        },
        ActivationQuantEntry {
            activation: ArtifactPath::new("activation.ffn_gelu")?,
            range: act_range,
            quant_format: ActivationQuantFormatSpec::Int8,
            eval_mode: ActivationEvalModeSpec::Quantized,
            nonlinearity: ActivationNonlinearitySpec::GeluClip,
        },
    ];

    let quant = QuantSpec::new_with_weight_quant(
        weight_entries,
        ternary_entries,
        activation_entries,
        norm_entries,
    );

    // Stateless bigram context: no artifact-schema variant exists, so record
    // the minimal linear-state placeholder (one f32 slot per layer).
    let sequence = SequenceSemanticsSpec::linear_state(LINEAR_STATE_SLOT_BYTES).map_err(|e| {
        CheckpointImportError::Manifest {
            reason: format!("sequence placeholder: {e}"),
        }
    })?;
    let sequence_placeholder = format!(
        "linear_state({LINEAR_STATE_SLOT_BYTES} bytes/layer) placeholder for stateless_bigram_context"
    );

    let core = ArtifactCore::new(tensors, quant, sequence)?;

    Ok(ImportedCheckpointExport {
        core,
        topology: ExportTopology {
            embedding: embedding_id,
            blocks: block_refs,
            d_model: d_model as usize,
            d_ff: d_ff as usize,
            vocab: vocab as usize,
        },
        facts: CheckpointExportFacts {
            schema: CHECKPOINT_EXPORT_SCHEMA.to_string(),
            trainer_git_sha: manifest.git_sha,
            manifest_sha256: sha256(&manifest_bytes).to_hex(),
            tensors_verified_sha256: tensors_verified,
            sequence_state_kind: manifest.topology.sequence_state_kind,
            sequence_semantics_placeholder: sequence_placeholder,
        },
    })
}

// ---------------------------------------------------------------------------
// synthetic export writer (tests / test-infra)
// ---------------------------------------------------------------------------

/// Write a complete synthetic checkpoint-export bundle (manifest + tensor
/// files with real sha256s) for tests that must not depend on the committed
/// experiment files. Mirrors the `f_s6_dense_checkpoint_export.v1` layout
/// exactly, sourced from [`gbf_kernel::model_ref::synthetic_checkpoint`].
#[cfg(any(test, feature = "test-infra"))]
pub fn write_synthetic_checkpoint_export(dir: &Path, seed: u64) -> std::io::Result<()> {
    use gbf_kernel::model_ref::{D_FF, D_MODEL, N_BLOCKS, VOCAB, synthetic_checkpoint};

    let ck = synthetic_checkpoint(seed);
    let tensors_dir = dir.join("tensors");
    std::fs::create_dir_all(&tensors_dir)?;

    let mut tensor_entries = Vec::new();
    let mut write_tensor = |name: &str,
                            file: &str,
                            dtype: &str,
                            shape: Vec<u32>,
                            bytes: Vec<u8>|
     -> std::io::Result<()> {
        std::fs::write(dir.join(file), &bytes)?;
        tensor_entries.push(serde_json::json!({
            "name": name,
            "file": file,
            "dtype": dtype,
            "layout": "row_major",
            "role": "synthetic",
            "shape": shape,
            "sha256": sha256(&bytes).to_hex(),
        }));
        Ok(())
    };

    let mut emb_bytes = Vec::with_capacity(VOCAB * D_MODEL * 4);
    for byte in 0..VOCAB {
        for &v in ck.embedding_row(byte as u8) {
            emb_bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    write_tensor(
        "embedding",
        "tensors/embedding.f32.bin",
        "f32_le",
        vec![VOCAB as u32, D_MODEL as u32],
        emb_bytes,
    )?;

    let mut layers = Vec::new();
    for (k, block) in ck.blocks().iter().enumerate() {
        for (proj, layer) in [("up", &block.up), ("down", &block.down)] {
            let mut weight_bytes = Vec::with_capacity(layer.rows() * layer.cols());
            let mut scale_bytes = Vec::with_capacity(layer.rows() * 2);
            for row in 0..layer.rows() {
                weight_bytes.extend(layer.row(row).iter().map(|&w| w as u8));
                scale_bytes.extend_from_slice(&layer.scale_raw(row).to_le_bytes());
            }
            write_tensor(
                &format!("block{k}_{proj}.ternary"),
                &format!("tensors/block{k}_{proj}.ternary.i8.bin"),
                "i8 (values in {-1,0,1})",
                vec![layer.rows() as u32, layer.cols() as u32],
                weight_bytes,
            )?;
            write_tensor(
                &format!("block{k}_{proj}.scales"),
                &format!("tensors/block{k}_{proj}.scales.q8_8_u16le.bin"),
                "u16_le (Q8.8 fixed-point; f32 = raw/256)",
                vec![layer.rows() as u32],
                scale_bytes,
            )?;
        }
        layers.push(serde_json::json!({
            "index": k,
            "kind": "prenorm_residual_ffn",
            "up_ternary": format!("block{k}_up.ternary"),
            "up_scales": format!("block{k}_up.scales"),
            "up_shape": [D_FF, D_MODEL],
            "down_ternary": format!("block{k}_down.ternary"),
            "down_scales": format!("block{k}_down.scales"),
            "down_shape": [D_MODEL, D_FF],
        }));
    }

    let manifest = serde_json::json!({
        "schema": CHECKPOINT_EXPORT_SCHEMA,
        "git_sha": format!("synthetic-seed-{seed}"),
        "seed": seed,
        "layers": layers,
        "tensors": tensor_entries,
        "numeric_convention": {
            "activation_fake_quant": {
                "format": "Int8_symmetric",
                "quant_steps": 127,
                "range_hi": 8.0,
                "range_lo": -8.0,
            },
            "block_forward": "x' = x + Down( gelu( Up( actq( rms_norm(x) ) ) ) ); logits = rms_norm(x_final) @ embedding^T",
            "embedding_dtype": "f32_le",
            "norm": {
                "affine_bias": 0.0,
                "affine_scale": 1.0,
                "clip_hi": 8.0,
                "clip_lo": -8.0,
                "epsilon": 9.999999747378752e-6,
                "kind": "tile_rms_then_affine_clip(full_vector)",
            },
            "weight_encoding": "Ternary2 {-1,0,+1}",
            "weight_scale": "per_output_row Q8.8 (u16 raw, f32 = raw/256)",
        },
        "topology": {
            "d_ff": D_FF,
            "d_model": D_MODEL,
            "family": "dense_ffn_bigram_context",
            "moe": false,
            "n_blocks": N_BLOCKS,
            "sequence_state_kind": "stateless_bigram_context",
            "sequence_state_params": {},
            "tied_head": true,
            "vocab": VOCAB,
        },
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_synthetic(seed: u64) -> ImportedCheckpointExport {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), seed).expect("writes export");
        import_checkpoint_export(dir.path()).expect("imports")
    }

    #[test]
    fn importer_builds_validated_core_from_synthetic_export() {
        let imported = import_synthetic(7);
        // 1 embedding + 4 blocks x (2 weights + 2 scales) = 17 tensors.
        assert_eq!(imported.core.tensors().len(), 17);
        assert_eq!(imported.facts.tensors_verified_sha256, 17);
        assert_eq!(imported.core.quant().ternary_weight_plans().len(), 8);
        // 8 ternary + 1 full-precision embedding entry.
        assert_eq!(imported.core.quant().weight_quant().len(), 9);
        assert_eq!(imported.core.quant().norm_plans().len(), 5);
        assert_eq!(imported.topology.blocks.len(), 4);
        assert_eq!(imported.topology.d_model, 64);
        assert_eq!(imported.topology.d_ff, 128);
        assert_eq!(imported.topology.vocab, 256);
        assert_eq!(
            imported.topology.blocks[2].up_projection.as_str(),
            "block2_up"
        );
        assert!(
            imported
                .facts
                .sequence_semantics_placeholder
                .contains("placeholder")
        );
    }

    #[test]
    fn importer_semantic_hash_is_deterministic() {
        let a = import_synthetic(3);
        let b = import_synthetic(3);
        let c = import_synthetic(4);
        assert_eq!(a.core.semantic_hash(), b.core.semantic_hash());
        assert_ne!(a.core.semantic_hash(), c.core.semantic_hash());
    }

    #[test]
    fn importer_rejects_corrupted_tensor_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 5).expect("writes export");
        let victim = dir.path().join("tensors/block1_up.ternary.i8.bin");
        let mut bytes = std::fs::read(&victim).expect("reads");
        bytes[0] = if bytes[0] == 1 { 0 } else { 1 };
        std::fs::write(&victim, bytes).expect("writes");
        let err = import_checkpoint_export(dir.path()).expect_err("must reject");
        assert!(
            matches!(err, CheckpointImportError::ShaMismatch { ref tensor, .. } if tensor == "block1_up.ternary"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn importer_rejects_wrong_schema() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 5).expect("writes export");
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("reads")).expect("parses");
        manifest["schema"] = serde_json::Value::String("something_else.v9".to_string());
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serializes"),
        )
        .expect("writes");
        let err = import_checkpoint_export(dir.path()).expect_err("must reject");
        assert!(matches!(
            err,
            CheckpointImportError::UnsupportedSchema { .. }
        ));
    }

    #[test]
    fn importer_rejects_unsupported_activation_range() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_synthetic_checkpoint_export(dir.path(), 5).expect("writes export");
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("reads")).expect("parses");
        manifest["numeric_convention"]["activation_fake_quant"]["range_hi"] =
            serde_json::json!(4.0);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serializes"),
        )
        .expect("writes");
        let err = import_checkpoint_export(dir.path()).expect_err("must reject");
        assert!(matches!(
            err,
            CheckpointImportError::UnsupportedConvention {
                field: "activation_fake_quant.range",
                ..
            }
        ));
    }
}
