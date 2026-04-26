//! Target-independent semantic artifact core.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;

use crate::ids::ArtifactPath;
use crate::norm_plan::{AffineClipLutPlan, NormPlan, TileRmsThenAffineClipPlan};
use crate::quant::{
    ActivationEvalModeSpec, ActivationQuantEntry, ActivationQuantFormatSpec,
    ActivationRangeModeSpec, ActivationRangeSpec, NormQuantEntry, QuantSpec, TernaryQuantEntry,
};
use crate::tensor::{
    CanonicalTensor, CanonicalTensorId, CanonicalTensorKind, TensorElementType, stable_digest,
};
use crate::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtifactCore {
    tensors: Vec<CanonicalTensor>,
    quant: QuantSpec,
}

impl ArtifactCore {
    pub fn new(
        mut tensors: Vec<CanonicalTensor>,
        quant: QuantSpec,
    ) -> Result<Self, ArtifactCoreError> {
        tensors.sort_by(|left, right| left.id.cmp(&right.id));
        let tensor_by_id = tensor_index(&tensors)?;
        let quant = quant.canonicalized();
        validate_quant_spec(&quant, &tensor_by_id)?;
        drop(tensor_by_id);

        Ok(Self { tensors, quant })
    }

    pub fn tensors(&self) -> &[CanonicalTensor] {
        &self.tensors
    }

    pub fn quant(&self) -> &QuantSpec {
        &self.quant
    }

    pub fn semantic_hash(&self) -> Hash256 {
        stable_digest(&artifact_core_semantic_bytes(&self.tensors, &self.quant))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactCoreError {
    DuplicateTensor {
        id: CanonicalTensorId,
    },
    DuplicateQuantEntry {
        kind: &'static str,
        path: ArtifactPath,
    },
    MissingTensor {
        role: &'static str,
        id: CanonicalTensorId,
    },
    TensorKindMismatch {
        id: CanonicalTensorId,
        expected: CanonicalTensorKind,
        actual: CanonicalTensorKind,
    },
    TensorElementTypeMismatch {
        id: CanonicalTensorId,
        expected: TensorElementType,
        actual: TensorElementType,
    },
    TensorRankMismatch {
        id: CanonicalTensorId,
        expected: usize,
        actual: usize,
    },
    TensorShapeMismatch {
        id: CanonicalTensorId,
        expected: Vec<u32>,
        actual: Vec<u32>,
    },
    InvalidActivationRange {
        activation: ArtifactPath,
    },
    InvalidNormPlan {
        norm: ArtifactPath,
        reason: &'static str,
    },
    MissingNormLut {
        norm: ArtifactPath,
    },
    UnexpectedNormLut {
        norm: ArtifactPath,
        lut: CanonicalTensorId,
    },
    InvalidQuantPlan {
        path: ArtifactPath,
        reason: &'static str,
    },
}

impl fmt::Display for ArtifactCoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTensor { id } => {
                write!(f, "artifact core contains duplicate tensor id {id}")
            }
            Self::DuplicateQuantEntry { kind, path } => {
                write!(f, "artifact core contains duplicate {kind} entry {path}")
            }
            Self::MissingTensor { role, id } => {
                write!(f, "artifact core {role} references missing tensor {id}")
            }
            Self::TensorKindMismatch {
                id,
                expected,
                actual,
            } => write!(f, "tensor {id} has kind {actual:?}, expected {expected:?}"),
            Self::TensorElementTypeMismatch {
                id,
                expected,
                actual,
            } => write!(
                f,
                "tensor {id} has element type {actual:?}, expected {expected:?}"
            ),
            Self::TensorRankMismatch {
                id,
                expected,
                actual,
            } => write!(f, "tensor {id} has rank {actual}, expected {expected}"),
            Self::TensorShapeMismatch {
                id,
                expected,
                actual,
            } => write!(f, "tensor {id} has shape {actual:?}, expected {expected:?}"),
            Self::InvalidActivationRange { activation } => {
                write!(f, "activation {activation} has invalid export range")
            }
            Self::InvalidNormPlan { norm, reason } => {
                write!(f, "norm {norm} has invalid export plan: {reason}")
            }
            Self::MissingNormLut { norm } => {
                write!(f, "norm {norm} requires a LUT tensor")
            }
            Self::UnexpectedNormLut { norm, lut } => {
                write!(f, "norm {norm} must not reference LUT tensor {lut}")
            }
            Self::InvalidQuantPlan { path, reason } => {
                write!(f, "quant plan {path} is invalid: {reason}")
            }
        }
    }
}

impl Error for ArtifactCoreError {}

fn tensor_index(
    tensors: &[CanonicalTensor],
) -> Result<BTreeMap<CanonicalTensorId, &CanonicalTensor>, ArtifactCoreError> {
    let mut by_id = BTreeMap::new();
    for tensor in tensors {
        if by_id.insert(tensor.id.clone(), tensor).is_some() {
            return Err(ArtifactCoreError::DuplicateTensor {
                id: tensor.id.clone(),
            });
        }
    }

    Ok(by_id)
}

fn validate_quant_spec(
    quant: &QuantSpec,
    tensors: &BTreeMap<CanonicalTensorId, &CanonicalTensor>,
) -> Result<(), ArtifactCoreError> {
    let mut ternary_entries = BTreeSet::new();
    for entry in quant.ternary_weight_plans() {
        if !ternary_entries.insert(entry.projection.clone()) {
            return Err(ArtifactCoreError::DuplicateQuantEntry {
                kind: "ternary projection",
                path: entry.projection.clone(),
            });
        }
        validate_ternary_entry(entry, tensors)?;
    }

    let mut activation_entries = BTreeSet::new();
    for entry in quant.activation_quant() {
        if !activation_entries.insert(entry.activation.clone()) {
            return Err(ArtifactCoreError::DuplicateQuantEntry {
                kind: "activation",
                path: entry.activation.clone(),
            });
        }
        validate_activation_entry(entry)?;
    }

    let mut norm_entries = BTreeSet::new();
    for entry in quant.norm_plans() {
        if !norm_entries.insert(entry.norm.clone()) {
            return Err(ArtifactCoreError::DuplicateQuantEntry {
                kind: "norm",
                path: entry.norm.clone(),
            });
        }
        validate_norm_entry(entry, tensors)?;
    }

    Ok(())
}

fn validate_ternary_entry(
    entry: &TernaryQuantEntry,
    tensors: &BTreeMap<CanonicalTensorId, &CanonicalTensor>,
) -> Result<(), ArtifactCoreError> {
    let weight = require_tensor(tensors, &entry.weight, "ternary weight")?;
    expect_tensor(
        weight,
        CanonicalTensorKind::TernaryWeight,
        TensorElementType::TernaryI2,
    )?;
    expect_rank(weight, 2)?;
    let weight_shape = weight.layout.shape.dims();
    let rows = weight_shape[0];
    let cols = weight_shape[1];

    let scale = require_tensor(tensors, &entry.scale, "ternary scale")?;
    expect_tensor(
        scale,
        CanonicalTensorKind::TernaryScale,
        TensorElementType::Q8_8,
    )?;
    expect_shape(scale, expected_scale_shape(entry, rows, cols)?)?;

    if let Some(bias_id) = &entry.bias {
        let bias = require_tensor(tensors, bias_id, "ternary bias")?;
        expect_tensor(bias, CanonicalTensorKind::Bias, TensorElementType::Float32)?;
        expect_shape(bias, vec![rows])?;
    }

    Ok(())
}

fn expected_scale_shape(
    entry: &TernaryQuantEntry,
    rows: u32,
    cols: u32,
) -> Result<Vec<u32>, ArtifactCoreError> {
    let scale_count = match entry.plan.scale_granularity {
        ScaleGranularity::PerTensor => 1_u128,
        ScaleGranularity::PerOutputRow => u128::from(rows),
        ScaleGranularity::PerGroup(group_size) => {
            (u128::from(rows) * u128::from(cols)).div_ceil(u128::from(group_size.get()))
        }
    };
    let scale_count =
        u32::try_from(scale_count).map_err(|_| ArtifactCoreError::InvalidQuantPlan {
            path: entry.projection.clone(),
            reason: "scale tensor shape exceeds u32",
        })?;

    Ok(vec![scale_count])
}

fn validate_activation_entry(entry: &ActivationQuantEntry) -> Result<(), ArtifactCoreError> {
    if !valid_range(entry.range) {
        return Err(ArtifactCoreError::InvalidActivationRange {
            activation: entry.activation.clone(),
        });
    }

    Ok(())
}

fn validate_norm_entry(
    entry: &NormQuantEntry,
    tensors: &BTreeMap<CanonicalTensorId, &CanonicalTensor>,
) -> Result<(), ArtifactCoreError> {
    match &entry.plan {
        NormPlan::AffineClipLut(plan) => {
            validate_affine_clip_lut_plan(&entry.norm, plan)?;
            let lut_id = entry
                .lut
                .as_ref()
                .ok_or_else(|| ArtifactCoreError::MissingNormLut {
                    norm: entry.norm.clone(),
                })?;
            let lut = require_tensor(tensors, lut_id, "norm lut")?;
            expect_tensor(
                lut,
                CanonicalTensorKind::NormLut,
                TensorElementType::Float32,
            )?;
            expect_shape(lut, vec![u32::from(plan.lut.entries)])?;
        }
        NormPlan::TileRmsThenAffineClip(plan) => {
            validate_tile_rms_plan(&entry.norm, plan)?;
            if let Some(lut) = &entry.lut {
                return Err(ArtifactCoreError::UnexpectedNormLut {
                    norm: entry.norm.clone(),
                    lut: lut.clone(),
                });
            }
        }
    }

    Ok(())
}

fn validate_affine_clip_lut_plan(
    norm: &ArtifactPath,
    plan: &AffineClipLutPlan,
) -> Result<(), ArtifactCoreError> {
    if !plan.affine.scale.is_finite() || !plan.affine.bias.is_finite() {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "affine params must be finite",
        });
    }
    if !plan.clip.lo.is_finite() || !plan.clip.hi.is_finite() || plan.clip.lo >= plan.clip.hi {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "clip bounds must be finite and ordered",
        });
    }
    if !plan.lut.input_lo.is_finite()
        || !plan.lut.input_hi.is_finite()
        || plan.lut.input_lo >= plan.lut.input_hi
        || plan.lut.entries < 2
    {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "lut bounds must be finite and ordered with at least two entries",
        });
    }

    Ok(())
}

fn validate_tile_rms_plan(
    norm: &ArtifactPath,
    plan: &TileRmsThenAffineClipPlan,
) -> Result<(), ArtifactCoreError> {
    if plan.tile.tile_width == 0 || !plan.tile.epsilon.is_finite() || plan.tile.epsilon <= 0.0 {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "tile width and epsilon must be valid",
        });
    }
    if !plan.affine.scale.is_finite() || !plan.affine.bias.is_finite() {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "affine params must be finite",
        });
    }
    if !plan.clip.lo.is_finite() || !plan.clip.hi.is_finite() || plan.clip.lo >= plan.clip.hi {
        return Err(ArtifactCoreError::InvalidNormPlan {
            norm: norm.clone(),
            reason: "clip bounds must be finite and ordered",
        });
    }

    Ok(())
}

fn require_tensor<'a>(
    tensors: &'a BTreeMap<CanonicalTensorId, &CanonicalTensor>,
    id: &CanonicalTensorId,
    role: &'static str,
) -> Result<&'a CanonicalTensor, ArtifactCoreError> {
    tensors
        .get(id)
        .copied()
        .ok_or_else(|| ArtifactCoreError::MissingTensor {
            role,
            id: id.clone(),
        })
}

fn expect_tensor(
    tensor: &CanonicalTensor,
    expected_kind: CanonicalTensorKind,
    expected_type: TensorElementType,
) -> Result<(), ArtifactCoreError> {
    if tensor.kind != expected_kind {
        return Err(ArtifactCoreError::TensorKindMismatch {
            id: tensor.id.clone(),
            expected: expected_kind,
            actual: tensor.kind,
        });
    }
    if tensor.layout.element_type != expected_type {
        return Err(ArtifactCoreError::TensorElementTypeMismatch {
            id: tensor.id.clone(),
            expected: expected_type,
            actual: tensor.layout.element_type,
        });
    }

    Ok(())
}

fn expect_rank(tensor: &CanonicalTensor, expected: usize) -> Result<(), ArtifactCoreError> {
    let actual = tensor.layout.shape.dims().len();
    if actual != expected {
        return Err(ArtifactCoreError::TensorRankMismatch {
            id: tensor.id.clone(),
            expected,
            actual,
        });
    }

    Ok(())
}

fn expect_shape(tensor: &CanonicalTensor, expected: Vec<u32>) -> Result<(), ArtifactCoreError> {
    let actual = tensor.layout.shape.dims().to_vec();
    if actual != expected {
        return Err(ArtifactCoreError::TensorShapeMismatch {
            id: tensor.id.clone(),
            expected,
            actual,
        });
    }

    Ok(())
}

fn valid_range(range: ActivationRangeSpec) -> bool {
    range.lo.is_finite() && range.hi.is_finite() && range.lo < range.hi
}

fn artifact_core_semantic_bytes(tensors: &[CanonicalTensor], quant: &QuantSpec) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_bytes(&mut bytes, b"gbf.artifact.core.v1");

    push_u64(&mut bytes, tensors.len() as u64);
    for tensor in tensors {
        push_path(&mut bytes, &tensor.id);
        push_tensor_kind(&mut bytes, tensor.kind);
        push_hash(&mut bytes, tensor.content_hash);
    }

    push_u64(&mut bytes, quant.ternary_weight_plans().len() as u64);
    for entry in quant.ternary_weight_plans() {
        push_ternary_quant_entry(&mut bytes, entry);
    }

    push_u64(&mut bytes, quant.activation_quant().len() as u64);
    for entry in quant.activation_quant() {
        push_activation_quant_entry(&mut bytes, entry);
    }

    push_u64(&mut bytes, quant.norm_plans().len() as u64);
    for entry in quant.norm_plans() {
        push_norm_quant_entry(&mut bytes, entry);
    }

    bytes
}

fn push_ternary_quant_entry(bytes: &mut Vec<u8>, entry: &TernaryQuantEntry) {
    push_path(bytes, &entry.projection);
    push_path(bytes, &entry.weight);
    push_path(bytes, &entry.scale);
    push_optional_path(bytes, entry.bias.as_ref());
    push_ternary_weight_plan(bytes, entry.plan);
}

fn push_activation_quant_entry(bytes: &mut Vec<u8>, entry: &ActivationQuantEntry) {
    push_path(bytes, &entry.activation);
    push_activation_range(bytes, entry.range);
    push_activation_quant_format(bytes, entry.quant_format);
    push_activation_eval_mode(bytes, entry.eval_mode);
}

fn push_norm_quant_entry(bytes: &mut Vec<u8>, entry: &NormQuantEntry) {
    push_path(bytes, &entry.norm);
    push_norm_plan(bytes, &entry.plan);
    push_optional_path(bytes, entry.lut.as_ref());
}

fn push_ternary_weight_plan(bytes: &mut Vec<u8>, plan: TernaryWeightPlan) {
    push_weight_encoding(bytes, plan.encoding);
    push_scale_granularity(bytes, plan.scale_granularity);
    push_scale_format(bytes, plan.scale_format);
    push_threshold_plan(bytes, plan.threshold);
}

fn push_norm_plan(bytes: &mut Vec<u8>, plan: &NormPlan) {
    match plan {
        NormPlan::AffineClipLut(plan) => {
            push_u8(bytes, 0);
            push_f32(bytes, plan.affine.scale);
            push_f32(bytes, plan.affine.bias);
            push_f32(bytes, plan.clip.lo);
            push_f32(bytes, plan.clip.hi);
            push_f32(bytes, plan.lut.input_lo);
            push_f32(bytes, plan.lut.input_hi);
            push_u16(bytes, plan.lut.entries);
        }
        NormPlan::TileRmsThenAffineClip(plan) => {
            push_u8(bytes, 1);
            push_u16(bytes, plan.tile.tile_width);
            push_f32(bytes, plan.tile.epsilon);
            push_f32(bytes, plan.affine.scale);
            push_f32(bytes, plan.affine.bias);
            push_f32(bytes, plan.clip.lo);
            push_f32(bytes, plan.clip.hi);
        }
    }
}

fn push_activation_range(bytes: &mut Vec<u8>, range: ActivationRangeSpec) {
    push_f32(bytes, range.lo);
    push_f32(bytes, range.hi);
    push_activation_range_mode(bytes, range.mode);
}

fn push_weight_encoding(bytes: &mut Vec<u8>, encoding: WeightEncoding) {
    let tag = match encoding {
        WeightEncoding::Ternary2 => 0,
        WeightEncoding::SparseTernaryBitplanes => 1,
        WeightEncoding::Binary1 => 2,
    };
    push_u8(bytes, tag);
}

fn push_scale_granularity(bytes: &mut Vec<u8>, granularity: ScaleGranularity) {
    match granularity {
        ScaleGranularity::PerTensor => push_u8(bytes, 0),
        ScaleGranularity::PerOutputRow => push_u8(bytes, 1),
        ScaleGranularity::PerGroup(group_size) => {
            push_u8(bytes, 2);
            push_u16(bytes, group_size.get());
        }
    }
}

fn push_scale_format(bytes: &mut Vec<u8>, format: ScaleFormat) {
    let tag = match format {
        ScaleFormat::Q8_8 => 0,
        ScaleFormat::Q4_4 => 1,
        ScaleFormat::Pow2 => 2,
    };
    push_u8(bytes, tag);
}

fn push_threshold_plan(bytes: &mut Vec<u8>, plan: ThresholdPlan) {
    match plan {
        ThresholdPlan::FixedQ8_8 => push_u8(bytes, 0),
        ThresholdPlan::AnnealedGlobalThenPerOutputRow => push_u8(bytes, 1),
        ThresholdPlan::LearnedPerGroup(group_size) => {
            push_u8(bytes, 2);
            push_u16(bytes, group_size.get());
        }
    }
}

fn push_activation_quant_format(bytes: &mut Vec<u8>, format: ActivationQuantFormatSpec) {
    let tag = match format {
        ActivationQuantFormatSpec::Int8 => 0,
        ActivationQuantFormatSpec::UInt8 => 1,
        ActivationQuantFormatSpec::UInt4 => 2,
    };
    push_u8(bytes, tag);
}

fn push_activation_range_mode(bytes: &mut Vec<u8>, mode: ActivationRangeModeSpec) {
    let tag = match mode {
        ActivationRangeModeSpec::Fixed => 0,
        ActivationRangeModeSpec::Learned => 1,
        ActivationRangeModeSpec::Ema => 2,
    };
    push_u8(bytes, tag);
}

fn push_activation_eval_mode(bytes: &mut Vec<u8>, mode: ActivationEvalModeSpec) {
    let tag = match mode {
        ActivationEvalModeSpec::Quantized => 0,
        ActivationEvalModeSpec::Passthrough => 1,
    };
    push_u8(bytes, tag);
}

fn push_tensor_kind(bytes: &mut Vec<u8>, kind: CanonicalTensorKind) {
    let tag = match kind {
        CanonicalTensorKind::TernaryWeight => 0,
        CanonicalTensorKind::TernaryScale => 1,
        CanonicalTensorKind::Bias => 2,
        CanonicalTensorKind::RouterWeight => 3,
        CanonicalTensorKind::RouterBias => 4,
        CanonicalTensorKind::DenseWeight => 5,
        CanonicalTensorKind::DenseBias => 6,
        CanonicalTensorKind::Embedding => 7,
        CanonicalTensorKind::Classifier => 8,
        CanonicalTensorKind::NormLut => 9,
        CanonicalTensorKind::SharedDenseAlpha => 10,
    };
    push_u8(bytes, tag);
}

fn push_optional_path(bytes: &mut Vec<u8>, path: Option<&ArtifactPath>) {
    match path {
        Some(path) => {
            push_u8(bytes, 1);
            push_path(bytes, path);
        }
        None => push_u8(bytes, 0),
    }
}

fn push_path(bytes: &mut Vec<u8>, path: &ArtifactPath) {
    push_bytes(bytes, path.as_str().as_bytes());
}

fn push_hash(bytes: &mut Vec<u8>, hash: Hash256) {
    bytes.extend_from_slice(hash.as_bytes());
}

fn push_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
    push_u64(bytes, value.len() as u64);
    bytes.extend_from_slice(value);
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn push_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use crate::quant::{
        ActivationQuantFormatSpec, ActivationRangeModeSpec, ActivationRangeSpec, TernaryQuantEntry,
    };
    use crate::tensor::{
        CanonicalTensor, CanonicalTensorId, CanonicalTensorKind, CanonicalTensorLayout,
        CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
    };

    use super::*;

    #[test]
    fn artifact_core_hash_is_deterministic_for_same_payload() {
        let core_a =
            ArtifactCore::new(vec![fixture_tensor("layer.0.weight")], QuantSpec::default())
                .unwrap();
        let core_b =
            ArtifactCore::new(vec![fixture_tensor("layer.0.weight")], QuantSpec::default())
                .unwrap();

        assert_eq!(core_a.semantic_hash(), core_b.semantic_hash());
    }

    #[test]
    fn artifact_core_rejects_duplicate_tensor_ids() {
        let err = ArtifactCore::new(
            vec![
                fixture_tensor("layer.0.weight"),
                fixture_tensor("layer.0.weight"),
            ],
            QuantSpec::default(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ArtifactCoreError::DuplicateTensor {
                id: CanonicalTensorId::new("layer.0.weight").unwrap()
            }
        );
    }

    #[test]
    fn artifact_core_rejects_missing_quant_tensor_references() {
        let quant = QuantSpec::new(
            vec![TernaryQuantEntry {
                projection: ArtifactPath::new("projection").unwrap(),
                weight: CanonicalTensorId::new("projection.weight").unwrap(),
                scale: CanonicalTensorId::new("projection.scale").unwrap(),
                bias: None,
                plan: TernaryWeightPlan::new(
                    WeightEncoding::Ternary2,
                    ScaleGranularity::PerOutputRow,
                    ScaleFormat::Q8_8,
                    ThresholdPlan::FixedQ8_8,
                ),
            }],
            vec![],
            vec![],
        );

        let err = ArtifactCore::new(vec![], quant).unwrap_err();

        assert_eq!(
            err,
            ArtifactCoreError::MissingTensor {
                role: "ternary weight",
                id: CanonicalTensorId::new("projection.weight").unwrap()
            }
        );
    }

    #[test]
    fn artifact_core_rejects_wrong_quant_tensor_kind() {
        let weight = float_tensor(
            "projection.weight",
            CanonicalTensorKind::RouterWeight,
            &[2, 2],
            vec![0.0; 4],
        );
        let scale = q8_8_tensor("projection.scale", &[2], vec![256, 256]);
        let quant = fixture_ternary_quant();

        let err = ArtifactCore::new(vec![weight, scale], quant).unwrap_err();

        assert_eq!(
            err,
            ArtifactCoreError::TensorKindMismatch {
                id: CanonicalTensorId::new("projection.weight").unwrap(),
                expected: CanonicalTensorKind::TernaryWeight,
                actual: CanonicalTensorKind::RouterWeight
            }
        );
    }

    #[test]
    fn artifact_core_rejects_wrong_scale_shape() {
        let weight = ternary_tensor("projection.weight", &[2, 2], vec![1, 0, -1, 1]);
        let scale = q8_8_tensor("projection.scale", &[3], vec![256, 256, 256]);
        let quant = fixture_ternary_quant();

        let err = ArtifactCore::new(vec![weight, scale], quant).unwrap_err();

        assert_eq!(
            err,
            ArtifactCoreError::TensorShapeMismatch {
                id: CanonicalTensorId::new("projection.scale").unwrap(),
                expected: vec![2],
                actual: vec![3]
            }
        );
    }

    #[test]
    fn artifact_core_activation_ranges_are_part_of_identity() {
        let base = activation_core(-1.0, 1.0);
        let changed = activation_core(-1.0, 2.0);

        assert_ne!(base.semantic_hash(), changed.semantic_hash());
    }

    #[test]
    fn artifact_core_canonical_hash_preserves_float_sign_bits() {
        let positive = ArtifactCore::new(
            vec![float_tensor(
                "projection.bias",
                CanonicalTensorKind::Bias,
                &[1],
                vec![0.0],
            )],
            QuantSpec::default(),
        )
        .unwrap();
        let negative = ArtifactCore::new(
            vec![float_tensor(
                "projection.bias",
                CanonicalTensorKind::Bias,
                &[1],
                vec![-0.0],
            )],
            QuantSpec::default(),
        )
        .unwrap();

        assert_ne!(positive.semantic_hash(), negative.semantic_hash());
    }

    fn activation_core(lo: f32, hi: f32) -> ArtifactCore {
        ArtifactCore::new(
            vec![],
            QuantSpec::new(
                vec![],
                vec![ActivationQuantEntry {
                    activation: ArtifactPath::new("activation").unwrap(),
                    range: ActivationRangeSpec {
                        lo,
                        hi,
                        mode: ActivationRangeModeSpec::Fixed,
                    },
                    quant_format: ActivationQuantFormatSpec::Int8,
                    eval_mode: ActivationEvalModeSpec::Quantized,
                }],
                vec![],
            ),
        )
        .unwrap()
    }

    fn fixture_ternary_quant() -> QuantSpec {
        QuantSpec::new(
            vec![TernaryQuantEntry {
                projection: ArtifactPath::new("projection").unwrap(),
                weight: CanonicalTensorId::new("projection.weight").unwrap(),
                scale: CanonicalTensorId::new("projection.scale").unwrap(),
                bias: None,
                plan: TernaryWeightPlan::new(
                    WeightEncoding::Ternary2,
                    ScaleGranularity::PerOutputRow,
                    ScaleFormat::Q8_8,
                    ThresholdPlan::FixedQ8_8,
                ),
            }],
            vec![],
            vec![],
        )
    }

    fn fixture_tensor(id: &str) -> CanonicalTensor {
        ternary_tensor(id, &[1, 1], vec![1])
    }

    fn ternary_tensor(id: &str, dims: &[usize], values: Vec<i8>) -> CanonicalTensor {
        tensor(
            id,
            CanonicalTensorKind::TernaryWeight,
            dims,
            TensorElementType::TernaryI2,
            CanonicalTensorPayload::I8(values),
        )
    }

    fn q8_8_tensor(id: &str, dims: &[usize], values: Vec<u16>) -> CanonicalTensor {
        tensor(
            id,
            CanonicalTensorKind::TernaryScale,
            dims,
            TensorElementType::Q8_8,
            CanonicalTensorPayload::U16(values),
        )
    }

    fn float_tensor(
        id: &str,
        kind: CanonicalTensorKind,
        dims: &[usize],
        values: Vec<f32>,
    ) -> CanonicalTensor {
        tensor(
            id,
            kind,
            dims,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(values),
        )
    }

    fn tensor(
        id: &str,
        kind: CanonicalTensorKind,
        dims: &[usize],
        element_type: TensorElementType,
        payload: CanonicalTensorPayload,
    ) -> CanonicalTensor {
        CanonicalTensor::new(
            CanonicalTensorId::new(id).unwrap(),
            kind,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(dims).unwrap(),
                element_type,
            ),
            payload,
        )
        .unwrap()
    }
}
