//! Deterministic export visitor for backend-independent QAT modules.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_artifact::core::{ArtifactCore, ArtifactCoreError};
use gbf_artifact::export_facts::{ExportFacts, RangeDigest};
use gbf_artifact::ids::{ArtifactPath, ArtifactPathError};
use gbf_artifact::norm_plan::NormExportParams;
use gbf_artifact::quant::{
    ActivationEvalModeSpec, ActivationNonlinearitySpec, ActivationQuantEntry,
    ActivationQuantFormatSpec, ActivationRangeModeSpec, ActivationRangeSpec, NormQuantEntry,
    QuantSpec, TernaryQuantEntry, WeightQuantEntry,
};
use gbf_artifact::sequence::{SequenceExportFacts, SequenceSemanticsError, SequenceSemanticsSpec};
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorError, CanonicalTensorId, CanonicalTensorKind,
    CanonicalTensorLayout, CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
};
use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use super::{
    ActFakeQuant, ActivationQuantFormat, ActivationRangeModeKind, ClippedActivationKind,
    DenseBranchProjection, ExpertBlockQat, ExpertQat, NormApproxPlan, NormApproxQat,
    SharedDenseBranch, TernaryLinearQat, Top1RouterQat,
};
use crate::sequence::{BoundedKvBlock, LinearStateBlock};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedQatArtifact {
    pub core: ArtifactCore,
    pub facts: ExportFacts,
    pub visited_modules: Vec<VisitedModule>,
}

impl ExportedQatArtifact {
    pub fn artifact_core_hash(&self) -> Hash256 {
        self.core.semantic_hash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VisitedModule {
    pub path: ArtifactPath,
    pub kind: VisitedModuleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum VisitedModuleKind {
    TernaryLinear,
    Activation,
    Norm,
    Router,
    Expert,
    ExpertBlock,
    SharedDenseBranch,
    DenseBranchProjection,
    LinearStateBlock,
    BoundedKvBlock,
}

#[derive(Debug, Clone, Copy)]
pub enum QatModuleRef<'a> {
    TernaryLinear(&'a TernaryLinearQat),
    Activation(&'a ActFakeQuant),
    Norm(&'a NormApproxQat),
    Router(&'a Top1RouterQat),
    Expert(&'a ExpertQat),
    ExpertBlock(&'a ExpertBlockQat),
    SharedDenseBranch(&'a SharedDenseBranch),
    DenseBranchProjection(&'a DenseBranchProjection),
    LinearStateBlock(&'a LinearStateBlock),
    BoundedKvBlock(&'a BoundedKvBlock),
}

#[derive(Debug, Clone)]
pub struct ExportVisitor {
    sequence_facts: SequenceExportFacts,
    tensors: BTreeMap<CanonicalTensorId, CanonicalTensor>,
    weight_quant: BTreeMap<ArtifactPath, WeightQuantEntry>,
    ternary_weight_plans: BTreeMap<ArtifactPath, TernaryQuantEntry>,
    activation_quant: BTreeMap<ArtifactPath, ActivationQuantEntry>,
    norm_plans: BTreeMap<ArtifactPath, NormQuantEntry>,
    activation_ranges: BTreeMap<ArtifactPath, RangeDigest>,
    sequence_tensor_handles: BTreeSet<CanonicalTensorId>,
    visited_modules: BTreeSet<VisitedModule>,
}

impl ExportVisitor {
    pub fn new(sequence_facts: SequenceExportFacts) -> Self {
        Self {
            sequence_facts,
            tensors: BTreeMap::new(),
            weight_quant: BTreeMap::new(),
            ternary_weight_plans: BTreeMap::new(),
            activation_quant: BTreeMap::new(),
            norm_plans: BTreeMap::new(),
            activation_ranges: BTreeMap::new(),
            sequence_tensor_handles: BTreeSet::new(),
            visited_modules: BTreeSet::new(),
        }
    }

    pub fn sequence_semantics(&self) -> SequenceSemanticsSpec {
        self.sequence_facts.spec()
    }

    pub fn sequence_facts(&self) -> &SequenceExportFacts {
        &self.sequence_facts
    }

    pub fn visit_module(
        &mut self,
        prefix: &str,
        module: QatModuleRef<'_>,
    ) -> Result<(), ExportVisitorError> {
        match module {
            QatModuleRef::TernaryLinear(layer) => self.visit_ternary_linear(prefix, layer),
            QatModuleRef::Activation(activation) => self.visit_activation(prefix, activation),
            QatModuleRef::Norm(norm) => self.visit_norm(prefix, norm),
            QatModuleRef::Router(router) => self.visit_router(prefix, router),
            QatModuleRef::Expert(expert) => self.visit_expert(prefix, expert),
            QatModuleRef::ExpertBlock(block) => self.visit_expert_block(prefix, block),
            QatModuleRef::SharedDenseBranch(shared_dense) => {
                let path = artifact_path(prefix)?;
                self.visit_shared_dense_at(path, shared_dense)
            }
            QatModuleRef::DenseBranchProjection(projection) => {
                self.visit_dense_projection(prefix, projection)
            }
            QatModuleRef::LinearStateBlock(block) => self.visit_linear_state_block(prefix, block),
            QatModuleRef::BoundedKvBlock(block) => self.visit_bounded_kv_block(prefix, block),
        }
    }

    pub fn visit_ternary_linear(
        &mut self,
        prefix: &str,
        layer: &TernaryLinearQat,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_ternary_linear_at(path, layer)
    }

    pub fn visit_activation(
        &mut self,
        prefix: &str,
        activation: &ActFakeQuant,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_activation_at(path, activation)
    }

    pub fn visit_norm(
        &mut self,
        prefix: &str,
        norm: &NormApproxQat,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_norm_at(path, norm)
    }

    pub fn visit_router(
        &mut self,
        prefix: &str,
        router: &Top1RouterQat,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_router_at(path, router)
    }

    pub fn visit_expert(
        &mut self,
        prefix: &str,
        expert: &ExpertQat,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_expert_at(path, expert)
    }

    pub fn visit_expert_block(
        &mut self,
        prefix: &str,
        block: &ExpertBlockQat,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_expert_block_at(path, block)
    }

    pub fn visit_dense_projection(
        &mut self,
        prefix: &str,
        projection: &DenseBranchProjection,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_dense_projection_at(path, projection)
    }

    pub fn visit_linear_state_block(
        &mut self,
        prefix: &str,
        block: &LinearStateBlock,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_linear_state_block_at(path, block)
    }

    pub fn visit_bounded_kv_block(
        &mut self,
        prefix: &str,
        block: &BoundedKvBlock,
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.visit_bounded_kv_block_at(path, block)
    }

    pub fn visit_embedding(
        &mut self,
        prefix: &str,
        rows: usize,
        cols: usize,
        values: &[f32],
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.add_float_tensor(
            path.clone(),
            CanonicalTensorKind::Embedding,
            &[rows, cols],
            values,
        )?;
        self.add_weight_quant(WeightQuantEntry::full_precision(path.clone(), path))
    }

    pub fn visit_classifier(
        &mut self,
        prefix: &str,
        rows: usize,
        cols: usize,
        values: &[f32],
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.add_float_tensor(
            path.clone(),
            CanonicalTensorKind::Classifier,
            &[rows, cols],
            values,
        )?;
        self.add_weight_quant(WeightQuantEntry::full_precision(path.clone(), path))
    }

    pub fn finish(self) -> Result<ExportedQatArtifact, ExportVisitorError> {
        let sequence_facts =
            sequence_facts_with_handles(self.sequence_facts, self.sequence_tensor_handles)?;
        let quant = QuantSpec::new_with_weight_quant(
            self.weight_quant.into_values().collect(),
            self.ternary_weight_plans.into_values().collect(),
            self.activation_quant.into_values().collect(),
            self.norm_plans.into_values().collect(),
        );
        let core = ArtifactCore::new(
            self.tensors.into_values().collect(),
            quant,
            sequence_facts.spec(),
        )?;
        let facts = ExportFacts::new(
            self.activation_ranges.into_values().collect(),
            sequence_facts,
        );

        Ok(ExportedQatArtifact {
            core,
            facts,
            visited_modules: self.visited_modules.into_iter().collect(),
        })
    }

    fn visit_expert_block_at(
        &mut self,
        path: ArtifactPath,
        block: &ExpertBlockQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::ExpertBlock);

        let expert_root = path.join("expert")?;
        for (expert_index, expert) in block.experts().iter().enumerate() {
            let expert_path = expert_root.join(&expert_index.to_string())?;
            self.visit_expert_at(expert_path, expert)?;
        }

        if let Some(shared_dense) = block.shared_dense() {
            self.visit_shared_dense_at(path.join("shared_dense")?, shared_dense)?;
        }

        Ok(())
    }

    fn visit_linear_state_block_at(
        &mut self,
        path: ArtifactPath,
        block: &LinearStateBlock,
    ) -> Result<(), ExportVisitorError> {
        if self.sequence_facts.spec() != block.spec() {
            return Err(ExportVisitorError::SequenceSpecMismatch {
                expected: self.sequence_facts.spec(),
                actual: block.spec(),
            });
        }

        self.mark_visited(path.clone(), VisitedModuleKind::LinearStateBlock);

        let input_norm = path.join("input_norm")?;
        self.visit_norm_at(input_norm.clone(), block.input_norm())?;
        self.record_norm_tensor_handles(input_norm, block.input_norm())?;

        self.visit_activation_at(path.join("input_activation")?, block.input_activation())?;

        let input_to_state = path.join("input_to_state")?;
        self.visit_ternary_linear_at(input_to_state.clone(), block.input_to_state())?;
        self.record_ternary_tensor_handles(input_to_state, block.input_to_state())?;

        let state_to_output = path.join("state_to_output")?;
        self.visit_ternary_linear_at(state_to_output.clone(), block.state_to_output())?;
        self.record_ternary_tensor_handles(state_to_output, block.state_to_output())?;

        self.visit_activation_at(path.join("output_activation")?, block.output_activation())
    }

    fn visit_bounded_kv_block_at(
        &mut self,
        path: ArtifactPath,
        block: &BoundedKvBlock,
    ) -> Result<(), ExportVisitorError> {
        if self.sequence_facts.spec() != block.spec() {
            return Err(ExportVisitorError::SequenceSpecMismatch {
                expected: self.sequence_facts.spec(),
                actual: block.spec(),
            });
        }

        self.mark_visited(path.clone(), VisitedModuleKind::BoundedKvBlock);

        let input_norm = path.join("input_norm")?;
        self.visit_norm_at(input_norm.clone(), block.input_norm())?;
        self.record_norm_tensor_handles(input_norm, block.input_norm())?;

        self.visit_activation_at(path.join("input_activation")?, block.input_activation())?;

        let query_projection = path.join("query_projection")?;
        self.visit_ternary_linear_at(query_projection.clone(), block.query_projection())?;
        self.record_ternary_tensor_handles(query_projection, block.query_projection())?;

        let kv_projection = path.join("kv_projection")?;
        self.visit_ternary_linear_at(kv_projection.clone(), block.kv_projection())?;
        self.record_ternary_tensor_handles(kv_projection, block.kv_projection())?;

        let output_projection = path.join("output_projection")?;
        self.visit_ternary_linear_at(output_projection.clone(), block.output_projection())?;
        self.record_ternary_tensor_handles(output_projection, block.output_projection())?;

        self.visit_activation_at(path.join("output_activation")?, block.output_activation())
    }

    fn visit_expert_at(
        &mut self,
        path: ArtifactPath,
        expert: &ExpertQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Expert);
        self.visit_ternary_linear_at(path.join("up")?, expert.up_projection())?;
        self.visit_activation_with_nonlinearity_at(
            path.join("activation")?,
            expert.activation(),
            activation_nonlinearity(expert.clipped_activation().kind()),
        )?;
        self.visit_ternary_linear_at(path.join("down")?, expert.down_projection())
    }

    fn visit_shared_dense_at(
        &mut self,
        path: ArtifactPath,
        shared_dense: &SharedDenseBranch,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::SharedDenseBranch);
        self.visit_dense_projection_at(path.join("up")?, shared_dense.up_projection())?;
        self.visit_activation_at(path.join("activation")?, shared_dense.activation())?;
        self.visit_dense_projection_at(path.join("down")?, shared_dense.down_projection())?;
        self.add_float_tensor(
            path.join("alpha")?,
            CanonicalTensorKind::SharedDenseAlpha,
            &[1],
            &[shared_dense.alpha()],
        )
    }

    fn visit_ternary_linear_at(
        &mut self,
        path: ArtifactPath,
        layer: &TernaryLinearQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::TernaryLinear);

        let export = layer.export_canonical();
        let shape = export.shape();
        let weight_id = path.join("weight")?;
        let scale_id = path.join("scale")?;
        let bias_id = export
            .bias_values()
            .map(|_| path.join("bias"))
            .transpose()?;

        let ternary_values = export
            .ternary_values()
            .iter()
            .map(|value| value.as_i8())
            .collect::<Vec<_>>();
        self.add_tensor(
            weight_id.clone(),
            CanonicalTensorKind::TernaryWeight,
            &[shape.output_rows(), shape.input_cols()],
            TensorElementType::TernaryI2,
            CanonicalTensorPayload::I8(ternary_values),
        )?;

        let scale_values = export
            .scales()
            .iter()
            .map(|scale| scale.raw())
            .collect::<Vec<_>>();
        self.add_tensor(
            scale_id.clone(),
            CanonicalTensorKind::TernaryScale,
            &[shape.output_rows()],
            TensorElementType::Q8_8,
            CanonicalTensorPayload::U16(scale_values),
        )?;

        if let (Some(id), Some(values)) = (bias_id.clone(), export.bias_values()) {
            self.add_float_tensor(
                id,
                CanonicalTensorKind::Bias,
                &[shape.output_rows()],
                values,
            )?;
        }

        let entry = TernaryQuantEntry {
            projection: path.clone(),
            weight: weight_id,
            scale: scale_id,
            bias: bias_id,
            plan: export.plan(),
        };
        let weight_quant =
            WeightQuantEntry::ternary(entry.projection.clone(), entry.weight.clone(), entry.plan);
        insert_unique(
            &mut self.ternary_weight_plans,
            path,
            entry,
            "ternary projection",
        )?;
        self.add_weight_quant(weight_quant)?;

        Ok(())
    }

    fn visit_activation_at(
        &mut self,
        path: ArtifactPath,
        activation: &ActFakeQuant,
    ) -> Result<(), ExportVisitorError> {
        self.visit_activation_with_nonlinearity_at(
            path,
            activation,
            ActivationNonlinearitySpec::Identity,
        )
    }

    fn visit_activation_with_nonlinearity_at(
        &mut self,
        path: ArtifactPath,
        activation: &ActFakeQuant,
        nonlinearity: ActivationNonlinearitySpec,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Activation);

        let range = activation.export_range();
        let range = ActivationRangeSpec {
            lo: range.lo(),
            hi: range.hi(),
            mode: activation_range_mode(activation.range_mode().kind()),
        };
        let quant_format = activation_quant_format(activation.quant_format());
        let eval_mode = activation_eval_mode(activation.eval_passthrough());
        insert_unique(
            &mut self.activation_ranges,
            path.clone(),
            RangeDigest {
                activation: path.clone(),
                range,
                quant_format,
                eval_mode,
            },
            "activation range",
        )?;
        insert_unique(
            &mut self.activation_quant,
            path.clone(),
            ActivationQuantEntry {
                activation: path,
                range,
                quant_format,
                eval_mode,
                nonlinearity,
            },
            "activation quant entry",
        )?;

        Ok(())
    }

    fn visit_norm_at(
        &mut self,
        path: ArtifactPath,
        norm: &NormApproxQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Norm);

        let export = norm.export_norm_params();
        let lut = match export.params() {
            NormExportParams::AffineClipLut { lut_values, .. } => {
                let lut_id = path.join("lut")?;
                self.add_float_tensor(
                    lut_id.clone(),
                    CanonicalTensorKind::NormLut,
                    &[lut_values.len()],
                    lut_values,
                )?;
                Some(lut_id)
            }
            NormExportParams::TileRmsThenAffineClip { .. } => None,
        };

        let entry = NormQuantEntry {
            norm: path.clone(),
            plan: export.plan(),
            lut,
        };
        insert_unique(&mut self.norm_plans, path, entry, "norm entry")?;

        Ok(())
    }

    fn visit_router_at(
        &mut self,
        path: ArtifactPath,
        router: &Top1RouterQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Router);

        let shape = router.shape();
        let input_projection = path.join("input_projection")?;
        let input_weight = input_projection.join("weight")?;
        self.add_float_tensor(
            input_weight.clone(),
            CanonicalTensorKind::RouterWeight,
            &[shape.rank(), shape.d_model()],
            router.input_projection(),
        )?;
        self.add_weight_quant(WeightQuantEntry::full_precision(
            input_projection.clone(),
            input_weight,
        ))?;
        if let Some(bias) = router.input_bias() {
            self.add_float_tensor(
                input_projection.join("bias")?,
                CanonicalTensorKind::RouterBias,
                &[shape.rank()],
                bias,
            )?;
        }

        let expert_projection = path.join("expert_projection")?;
        let expert_weight = expert_projection.join("weight")?;
        self.add_float_tensor(
            expert_weight.clone(),
            CanonicalTensorKind::RouterWeight,
            &[shape.n_experts(), shape.rank()],
            router.expert_projection(),
        )?;
        self.add_weight_quant(WeightQuantEntry::full_precision(
            expert_projection.clone(),
            expert_weight,
        ))?;
        if let Some(bias) = router.expert_bias() {
            self.add_float_tensor(
                expert_projection.join("bias")?,
                CanonicalTensorKind::RouterBias,
                &[shape.n_experts()],
                bias,
            )?;
        }

        Ok(())
    }

    fn visit_dense_projection_at(
        &mut self,
        path: ArtifactPath,
        projection: &DenseBranchProjection,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::DenseBranchProjection);

        let shape = projection.shape();
        let weight = path.join("weight")?;
        self.add_float_tensor(
            weight.clone(),
            CanonicalTensorKind::DenseWeight,
            &[shape.output_rows(), shape.input_cols()],
            projection.weights(),
        )?;
        self.add_weight_quant(WeightQuantEntry::full_precision(path.clone(), weight))?;
        if let Some(bias) = projection.bias() {
            self.add_float_tensor(
                path.join("bias")?,
                CanonicalTensorKind::DenseBias,
                &[shape.output_rows()],
                bias,
            )?;
        }

        Ok(())
    }

    fn add_float_tensor(
        &mut self,
        id: CanonicalTensorId,
        kind: CanonicalTensorKind,
        dims: &[usize],
        values: &[f32],
    ) -> Result<(), ExportVisitorError> {
        self.add_tensor(
            id,
            kind,
            dims,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(values.to_vec()),
        )
    }

    fn add_tensor(
        &mut self,
        id: CanonicalTensorId,
        kind: CanonicalTensorKind,
        dims: &[usize],
        element_type: TensorElementType,
        payload: CanonicalTensorPayload,
    ) -> Result<(), ExportVisitorError> {
        let layout =
            CanonicalTensorLayout::new(CanonicalTensorShape::from_usize_dims(dims)?, element_type);
        let tensor = CanonicalTensor::new(id.clone(), kind, layout, payload)?;
        insert_unique(&mut self.tensors, id, tensor, "tensor")?;
        Ok(())
    }

    fn add_weight_quant(&mut self, entry: WeightQuantEntry) -> Result<(), ExportVisitorError> {
        insert_unique(
            &mut self.weight_quant,
            entry.weight.clone(),
            entry,
            "weight quant entry",
        )
    }

    fn mark_visited(&mut self, path: ArtifactPath, kind: VisitedModuleKind) {
        self.visited_modules.insert(VisitedModule { path, kind });
    }

    fn record_ternary_tensor_handles(
        &mut self,
        path: ArtifactPath,
        layer: &TernaryLinearQat,
    ) -> Result<(), ExportVisitorError> {
        self.sequence_tensor_handles.insert(path.join("weight")?);
        self.sequence_tensor_handles.insert(path.join("scale")?);
        if layer.bias().is_some() {
            self.sequence_tensor_handles.insert(path.join("bias")?);
        }
        Ok(())
    }

    fn record_norm_tensor_handles(
        &mut self,
        path: ArtifactPath,
        norm: &NormApproxQat,
    ) -> Result<(), ExportVisitorError> {
        if matches!(norm.plan(), NormApproxPlan::AffineClipLut { .. }) {
            self.sequence_tensor_handles.insert(path.join("lut")?);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportVisitorError {
    ArtifactPath(ArtifactPathError),
    Tensor(CanonicalTensorError),
    ArtifactCore(ArtifactCoreError),
    SequenceSemantics(SequenceSemanticsError),
    SequenceSpecMismatch {
        expected: SequenceSemanticsSpec,
        actual: SequenceSemanticsSpec,
    },
    DuplicatePath {
        kind: &'static str,
        path: ArtifactPath,
    },
}

impl fmt::Display for ExportVisitorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtifactPath(error) => write!(f, "{error}"),
            Self::Tensor(error) => write!(f, "{error}"),
            Self::ArtifactCore(error) => write!(f, "{error}"),
            Self::SequenceSemantics(error) => write!(f, "{error}"),
            Self::SequenceSpecMismatch { expected, actual } => write!(
                f,
                "sequence spec mismatch: visitor expected {expected:?}, block has {actual:?}"
            ),
            Self::DuplicatePath { kind, path } => {
                write!(f, "duplicate {kind} path {path}")
            }
        }
    }
}

impl Error for ExportVisitorError {}

impl From<ArtifactPathError> for ExportVisitorError {
    fn from(error: ArtifactPathError) -> Self {
        Self::ArtifactPath(error)
    }
}

impl From<CanonicalTensorError> for ExportVisitorError {
    fn from(error: CanonicalTensorError) -> Self {
        Self::Tensor(error)
    }
}

impl From<ArtifactCoreError> for ExportVisitorError {
    fn from(error: ArtifactCoreError) -> Self {
        Self::ArtifactCore(error)
    }
}

impl From<SequenceSemanticsError> for ExportVisitorError {
    fn from(error: SequenceSemanticsError) -> Self {
        Self::SequenceSemantics(error)
    }
}

fn artifact_path(value: &str) -> Result<ArtifactPath, ExportVisitorError> {
    Ok(ArtifactPath::new(value)?)
}

fn insert_unique<T>(
    values: &mut BTreeMap<ArtifactPath, T>,
    path: ArtifactPath,
    value: T,
    kind: &'static str,
) -> Result<(), ExportVisitorError> {
    if values.insert(path.clone(), value).is_some() {
        return Err(ExportVisitorError::DuplicatePath { kind, path });
    }

    Ok(())
}

fn sequence_facts_with_handles(
    facts: SequenceExportFacts,
    handles: BTreeSet<CanonicalTensorId>,
) -> Result<SequenceExportFacts, ExportVisitorError> {
    let mut all_handles = facts
        .canonical_tensor_handles()
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    all_handles.extend(handles);
    Ok(SequenceExportFacts::new(
        facts.spec(),
        facts.measured_state_size(),
        all_handles.into_iter().collect(),
    )?)
}

fn activation_quant_format(format: ActivationQuantFormat) -> ActivationQuantFormatSpec {
    match format {
        ActivationQuantFormat::Int8 => ActivationQuantFormatSpec::Int8,
        ActivationQuantFormat::UInt8 => ActivationQuantFormatSpec::UInt8,
        ActivationQuantFormat::UInt4 => ActivationQuantFormatSpec::UInt4,
    }
}

fn activation_range_mode(kind: ActivationRangeModeKind) -> ActivationRangeModeSpec {
    match kind {
        ActivationRangeModeKind::Fixed => ActivationRangeModeSpec::Fixed,
        ActivationRangeModeKind::Learned => ActivationRangeModeSpec::Learned,
        ActivationRangeModeKind::Ema => ActivationRangeModeSpec::Ema,
    }
}

fn activation_eval_mode(eval_passthrough: bool) -> ActivationEvalModeSpec {
    if eval_passthrough {
        ActivationEvalModeSpec::Passthrough
    } else {
        ActivationEvalModeSpec::Quantized
    }
}

fn activation_nonlinearity(kind: ClippedActivationKind) -> ActivationNonlinearitySpec {
    match kind {
        ClippedActivationKind::Relu => ActivationNonlinearitySpec::Relu,
        ClippedActivationKind::GeluClip => ActivationNonlinearitySpec::GeluClip,
        ClippedActivationKind::SiluClip => ActivationNonlinearitySpec::SiluClip,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_artifact::quant::{
        ActivationEvalModeSpec, ActivationNonlinearitySpec, ActivationRangeModeSpec,
    };
    use gbf_artifact::tensor::{CanonicalTensorKind, CanonicalTensorPayload, TensorElementType};

    use super::*;
    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
        ClippedActivation, DenseBranchProjection, EmaDecay, LutSpec, MatrixShape, NormApproxPlan,
        NormClip, Q8_8Scale, RouterShape, TernaryThreshold,
    };
    use crate::sequence::{BoundedKvBlockConfig, LinearStateBlockConfig, SequenceBlock};

    #[test]
    fn qat_export_visitor_builds_deterministic_artifact_for_same_modules() {
        let block_b = fixture_expert_block();
        let router_b = fixture_router();
        let norm_b = fixture_norm();

        let first = fixture_export();

        let mut second = export_visitor();
        second
            .visit_classifier("classifier", 2, 2, &[0.4, 0.3, 0.2, 0.1])
            .unwrap();
        second.visit_router("block.0.router", &router_b).unwrap();
        second.visit_norm("block.0.norm", &norm_b).unwrap();
        second
            .visit_embedding("token_embedding", 2, 2, &[0.1, 0.2, 0.3, 0.4])
            .unwrap();
        second.visit_expert_block("block.0.ffn", &block_b).unwrap();
        let second = second.finish().unwrap();

        assert_eq!(first.artifact_core_hash(), second.artifact_core_hash());
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
    }

    #[test]
    fn qat_export_visitor_has_committed_golden_core_hash() {
        let export = fixture_export();

        assert_eq!(
            export.artifact_core_hash().to_string(),
            "c1e77d89a82a888a53cfe9a8871fa1148dd4080228c486ea82ad3c16a6ce75f5"
        );
        assert_eq!(export.core.tensors().len(), 17);
        assert_eq!(export.facts.activation_ranges.len(), 2);
        assert_eq!(export.facts.sequence.spec(), fixture_sequence());
        assert_eq!(export.core.quant().weight_quant().len(), 8);
        assert_eq!(export.core.sequence_semantics(), fixture_sequence());
    }

    #[test]
    fn qat_export_visitor_records_all_qat_module_kinds_and_activation_ranges() {
        let block = fixture_expert_block();
        let router = fixture_router();
        let norm = fixture_norm();

        let mut visitor = export_visitor();
        visitor
            .visit_module("block.0.ffn", QatModuleRef::ExpertBlock(&block))
            .unwrap();
        visitor.visit_router("block.0.router", &router).unwrap();
        visitor.visit_norm("block.0.norm", &norm).unwrap();
        visitor
            .visit_activation("block.0.post_activation", &ema_activation())
            .unwrap();
        let export = visitor.finish().unwrap();

        let kinds = export
            .visited_modules
            .iter()
            .map(|module| module.kind)
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains(&VisitedModuleKind::TernaryLinear));
        assert!(kinds.contains(&VisitedModuleKind::Activation));
        assert!(kinds.contains(&VisitedModuleKind::Norm));
        assert!(kinds.contains(&VisitedModuleKind::Router));
        assert!(kinds.contains(&VisitedModuleKind::Expert));
        assert!(kinds.contains(&VisitedModuleKind::ExpertBlock));
        assert!(kinds.contains(&VisitedModuleKind::SharedDenseBranch));
        assert!(kinds.contains(&VisitedModuleKind::DenseBranchProjection));

        let range_paths = export
            .facts
            .activation_ranges
            .iter()
            .map(|range| range.activation.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            range_paths,
            vec![
                "block.0.ffn.expert.0.activation",
                "block.0.ffn.shared_dense.activation",
                "block.0.post_activation"
            ]
        );
        assert!(matches!(
            export.facts.activation_ranges.last().unwrap().range.mode,
            ActivationRangeModeSpec::Ema
        ));
    }

    #[test]
    fn qat_export_visitor_emits_scale_tensors_separately_from_ternary_weights() {
        let layer = TernaryLinearQat::new(
            MatrixShape::new(2, 3).unwrap(),
            vec![
                1.0, -1.0, 0.25, //
                0.75, -0.75, 0.0,
            ],
            None,
            vec![
                TernaryThreshold::new(0.5).unwrap(),
                TernaryThreshold::new(0.5).unwrap(),
            ],
            vec![
                Q8_8Scale::from_f32(0.5).unwrap(),
                Q8_8Scale::from_f32(2.0).unwrap(),
            ],
        )
        .unwrap();

        let mut visitor = export_visitor();
        visitor.visit_ternary_linear("projection", &layer).unwrap();
        let export = visitor.finish().unwrap();

        let tensor_ids = export
            .core
            .tensors()
            .iter()
            .map(|tensor| tensor.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(tensor_ids, vec!["projection.scale", "projection.weight"]);

        let weight = tensor(&export, "projection.weight");
        let scale = tensor(&export, "projection.scale");
        assert_eq!(weight.kind, CanonicalTensorKind::TernaryWeight);
        assert_eq!(scale.kind, CanonicalTensorKind::TernaryScale);
        assert_eq!(weight.layout.shape.dims(), &[2, 3]);
        assert_eq!(scale.layout.shape.dims(), &[2]);
        assert_eq!(scale.layout.element_type, TensorElementType::Q8_8);
        assert_eq!(
            scale.payload.as_u16_slice(),
            Some(
                &[
                    Q8_8Scale::from_f32(0.5).unwrap().raw(),
                    Q8_8Scale::from_f32(2.0).unwrap().raw(),
                ][..]
            )
        );
        assert_eq!(
            reconstruct_projected_weights(weight, scale),
            layer.export_canonical().projected_weights()
        );

        let quant = &export.core.quant().ternary_weight_plans()[0];
        assert_eq!(quant.weight.as_str(), "projection.weight");
        assert_eq!(quant.scale.as_str(), "projection.scale");
        assert!(quant.bias.is_none());
        assert_eq!(quant.plan, QuantSpec::default_expert_ternary_plan());

        let weight_quant = &export.core.quant().weight_quant()[0];
        assert_eq!(weight_quant.weight.as_str(), "projection");
        assert_eq!(weight_quant.tensor.as_str(), "projection.weight");
        assert_eq!(weight_quant.ternary_plan, Some(quant.plan));
    }

    #[test]
    fn qat_export_visitor_extracts_router_shared_dense_and_norm_lut_tensors() {
        let block = fixture_expert_block();
        let router = fixture_router();
        let norm = fixture_norm();

        let mut visitor = export_visitor();
        visitor.visit_expert_block("block.0.ffn", &block).unwrap();
        visitor.visit_router("block.0.router", &router).unwrap();
        visitor.visit_norm("block.0.norm", &norm).unwrap();
        let export = visitor.finish().unwrap();

        assert_eq!(
            tensor(&export, "block.0.router.input_projection.weight").kind,
            CanonicalTensorKind::RouterWeight
        );
        assert_eq!(
            tensor(&export, "block.0.ffn.shared_dense.alpha")
                .payload
                .as_f32_slice(),
            Some(&[0.25][..])
        );
        assert_eq!(
            tensor(&export, "block.0.norm.lut").payload,
            CanonicalTensorPayload::F32(vec![-1.0, -1.0, 1.0])
        );
        let full_precision_weights = export
            .core
            .quant()
            .weight_quant()
            .iter()
            .filter(|entry| entry.ternary_plan.is_none())
            .map(|entry| entry.weight.as_str())
            .collect::<BTreeSet<_>>();
        assert!(full_precision_weights.contains("block.0.router.input_projection"));
        assert!(full_precision_weights.contains("block.0.router.expert_projection"));
        assert!(full_precision_weights.contains("block.0.ffn.shared_dense.up"));
        assert!(full_precision_weights.contains("block.0.ffn.shared_dense.down"));

        let export = fixture_export();
        let embedding = weight_quant(&export, "token_embedding");
        assert_eq!(embedding.tensor.as_str(), "token_embedding");
        assert_eq!(embedding.ternary_plan, None);
        let classifier = weight_quant(&export, "classifier");
        assert_eq!(classifier.tensor.as_str(), "classifier");
        assert_eq!(classifier.ternary_plan, None);
    }

    #[test]
    fn qat_export_visitor_encodes_activation_eval_passthrough_in_core_quant() {
        let mut quantized = export_visitor();
        quantized
            .visit_activation("activation", &activation())
            .unwrap();
        let quantized = quantized.finish().unwrap();

        let mut passthrough = export_visitor();
        passthrough
            .visit_activation("activation", &activation().with_eval_passthrough(true))
            .unwrap();
        let passthrough = passthrough.finish().unwrap();

        assert_ne!(
            quantized.artifact_core_hash(),
            passthrough.artifact_core_hash()
        );
        assert_eq!(
            passthrough.core.quant().activation_quant()[0].eval_mode,
            ActivationEvalModeSpec::Passthrough
        );
    }

    #[test]
    fn qat_export_visitor_encodes_expert_clipped_activation_kind() {
        let relu = fixture_expert();
        let gelu = fixture_gelu_expert();

        let mut relu_export = export_visitor();
        relu_export.visit_expert("expert", &relu).unwrap();
        let relu_export = relu_export.finish().unwrap();

        let mut gelu_export = export_visitor();
        gelu_export.visit_expert("expert", &gelu).unwrap();
        let gelu_export = gelu_export.finish().unwrap();

        let relu_block = ExpertBlockQat::new(vec![relu.clone()], None).unwrap();
        let gelu_block = ExpertBlockQat::new(vec![gelu.clone()], None).unwrap();
        assert_ne!(
            relu_block.forward(&[2.0, -2.0], 0).unwrap(),
            gelu_block.forward(&[2.0, -2.0], 0).unwrap()
        );
        assert_ne!(
            relu_export.artifact_core_hash(),
            gelu_export.artifact_core_hash()
        );
        assert_eq!(
            relu_export.core.quant().activation_quant()[0].nonlinearity,
            ActivationNonlinearitySpec::Relu
        );
        assert_eq!(
            gelu_export.core.quant().activation_quant()[0].nonlinearity,
            ActivationNonlinearitySpec::GeluClip
        );
    }

    #[test]
    fn qat_export_visitor_records_sequence_export_facts() {
        let sequence_facts =
            SequenceExportFacts::for_spec(SequenceSemanticsSpec::linear_state(64).unwrap());
        let mut visitor = ExportVisitor::new(sequence_facts.clone());
        visitor
            .visit_activation("activation", &activation())
            .unwrap();

        let export = visitor.finish().unwrap();

        assert_eq!(export.core.sequence_semantics(), sequence_facts.spec());
        assert_eq!(export.facts.sequence, sequence_facts);
    }

    #[test]
    fn qat_export_visitor_walks_linear_state_block_and_records_real_sequence_handles() {
        let block = fixture_linear_state_block();
        let mut visitor = ExportVisitor::new(SequenceExportFacts::for_spec(block.spec()));
        visitor
            .visit_linear_state_block("block.0.sequence", &block)
            .unwrap();

        let export = visitor.finish().unwrap();
        let tensor_ids = export
            .core
            .tensors()
            .iter()
            .map(|tensor| tensor.id.clone())
            .collect::<BTreeSet<_>>();
        let handles = export
            .facts
            .sequence
            .canonical_tensor_handles()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(
            handles,
            vec![
                "block.0.sequence.input_norm.lut",
                "block.0.sequence.input_to_state.scale",
                "block.0.sequence.input_to_state.weight",
                "block.0.sequence.state_to_output.scale",
                "block.0.sequence.state_to_output.weight",
            ]
        );
        for handle in export.facts.sequence.canonical_tensor_handles() {
            assert!(tensor_ids.contains(handle), "missing tensor for {handle}");
        }
        assert!(
            export
                .visited_modules
                .iter()
                .any(|module| module.kind == VisitedModuleKind::LinearStateBlock)
        );
    }

    #[test]
    fn qat_export_visitor_rejects_linear_state_sequence_mismatch() {
        let block = fixture_linear_state_block();
        let mut visitor = export_visitor();
        let err = visitor
            .visit_linear_state_block("block.0.sequence", &block)
            .unwrap_err();

        assert!(matches!(
            err,
            ExportVisitorError::SequenceSpecMismatch { .. }
        ));
    }

    #[test]
    fn qat_export_visitor_walks_bounded_kv_block_and_records_real_sequence_handles() {
        let block = fixture_bounded_kv_block();
        let mut visitor = ExportVisitor::new(SequenceExportFacts::for_spec(block.spec()));
        visitor
            .visit_bounded_kv_block("block.0.sequence", &block)
            .unwrap();

        let export = visitor.finish().unwrap();
        let tensor_ids = export
            .core
            .tensors()
            .iter()
            .map(|tensor| tensor.id.clone())
            .collect::<BTreeSet<_>>();
        let handles = export
            .facts
            .sequence
            .canonical_tensor_handles()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(
            handles,
            vec![
                "block.0.sequence.input_norm.lut",
                "block.0.sequence.kv_projection.scale",
                "block.0.sequence.kv_projection.weight",
                "block.0.sequence.output_projection.scale",
                "block.0.sequence.output_projection.weight",
                "block.0.sequence.query_projection.scale",
                "block.0.sequence.query_projection.weight",
            ]
        );
        assert_eq!(export.core.sequence_semantics(), block.spec());
        assert_eq!(
            export.facts.sequence.measured_state_size(),
            block.state_size()
        );
        let activation_paths = export
            .core
            .quant()
            .activation_quant()
            .iter()
            .map(|entry| entry.activation.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            activation_paths,
            vec![
                "block.0.sequence.input_activation",
                "block.0.sequence.output_activation",
            ]
        );
        for handle in export.facts.sequence.canonical_tensor_handles() {
            assert!(tensor_ids.contains(handle), "missing tensor for {handle}");
        }
        assert!(
            export
                .visited_modules
                .iter()
                .any(|module| module.kind == VisitedModuleKind::BoundedKvBlock)
        );
    }

    #[test]
    fn qat_export_visitor_rejects_bounded_kv_sequence_mismatch() {
        let block = fixture_bounded_kv_block();
        let mut visitor = export_visitor();
        let err = visitor
            .visit_bounded_kv_block("block.0.sequence", &block)
            .unwrap_err();

        assert!(matches!(
            err,
            ExportVisitorError::SequenceSpecMismatch { .. }
        ));
    }

    #[test]
    fn qat_export_visitor_supports_each_qat_module_ref_variant() {
        let ternary = ternary_linear(1, 1, vec![1.0], None);
        let activation = activation();
        let norm = fixture_norm();
        let router = fixture_router();
        let expert = fixture_expert();
        let block = fixture_expert_block();
        let shared_dense = fixture_shared_dense();
        let dense =
            DenseBranchProjection::new(MatrixShape::new(1, 1).unwrap(), vec![1.0], None).unwrap();
        let linear_state = fixture_linear_state_block();
        let bounded_kv = fixture_bounded_kv_block();

        let mut visitor = ExportVisitor::new(SequenceExportFacts::for_spec(linear_state.spec()));
        visitor
            .visit_module("module.ternary", QatModuleRef::TernaryLinear(&ternary))
            .unwrap();
        visitor
            .visit_module("module.activation", QatModuleRef::Activation(&activation))
            .unwrap();
        visitor
            .visit_module("module.norm", QatModuleRef::Norm(&norm))
            .unwrap();
        visitor
            .visit_module("module.router", QatModuleRef::Router(&router))
            .unwrap();
        visitor
            .visit_module("module.expert", QatModuleRef::Expert(&expert))
            .unwrap();
        visitor
            .visit_module("module.block", QatModuleRef::ExpertBlock(&block))
            .unwrap();
        visitor
            .visit_module(
                "module.shared",
                QatModuleRef::SharedDenseBranch(&shared_dense),
            )
            .unwrap();
        visitor
            .visit_module("module.dense", QatModuleRef::DenseBranchProjection(&dense))
            .unwrap();
        visitor
            .visit_module(
                "module.linear_state",
                QatModuleRef::LinearStateBlock(&linear_state),
            )
            .unwrap();
        let export = visitor.finish().unwrap();

        let kinds = export
            .visited_modules
            .iter()
            .map(|module| module.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            kinds,
            BTreeSet::from([
                VisitedModuleKind::TernaryLinear,
                VisitedModuleKind::Activation,
                VisitedModuleKind::Norm,
                VisitedModuleKind::Router,
                VisitedModuleKind::Expert,
                VisitedModuleKind::ExpertBlock,
                VisitedModuleKind::SharedDenseBranch,
                VisitedModuleKind::DenseBranchProjection,
                VisitedModuleKind::LinearStateBlock,
            ])
        );

        let mut bounded_visitor =
            ExportVisitor::new(SequenceExportFacts::for_spec(bounded_kv.spec()));
        bounded_visitor
            .visit_module(
                "module.bounded_kv",
                QatModuleRef::BoundedKvBlock(&bounded_kv),
            )
            .unwrap();
        let bounded_export = bounded_visitor.finish().unwrap();

        assert!(
            bounded_export
                .visited_modules
                .iter()
                .any(|module| module.kind == VisitedModuleKind::BoundedKvBlock)
        );
    }

    #[test]
    fn qat_export_visitor_rejects_duplicate_activation_paths() {
        let activation = activation();
        let mut visitor = export_visitor();
        visitor
            .visit_activation("dup.activation", &activation)
            .unwrap();
        let err = visitor
            .visit_activation("dup.activation", &activation)
            .unwrap_err();

        assert_eq!(
            err,
            ExportVisitorError::DuplicatePath {
                kind: "activation range",
                path: ArtifactPath::new("dup.activation").unwrap()
            }
        );
    }

    #[test]
    fn qat_export_visitor_rejects_non_finite_raw_float_tensors() {
        let mut visitor = export_visitor();

        assert!(matches!(
            visitor.visit_embedding("embedding", 1, 1, &[f32::NAN]),
            Err(ExportVisitorError::Tensor(
                CanonicalTensorError::NonFiniteFloat { index: 0 }
            ))
        ));
    }

    fn tensor<'a>(export: &'a ExportedQatArtifact, id: &str) -> &'a CanonicalTensor {
        export
            .core
            .tensors()
            .iter()
            .find(|tensor| tensor.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing tensor {id}"))
    }

    fn reconstruct_projected_weights(
        weight: &CanonicalTensor,
        scale: &CanonicalTensor,
    ) -> Vec<f32> {
        let rows = weight.layout.shape.dims()[0] as usize;
        let cols = weight.layout.shape.dims()[1] as usize;
        let weights = weight.payload.as_i8_slice().unwrap();
        let scales = scale.payload.as_u16_slice().unwrap();

        assert_eq!(scales.len(), rows);
        weights
            .chunks_exact(cols)
            .zip(scales)
            .flat_map(|(row, &scale)| {
                let scale = Q8_8Scale::from_raw(scale).to_f32();
                row.iter().map(move |&value| f32::from(value) * scale)
            })
            .collect()
    }

    fn weight_quant<'a>(export: &'a ExportedQatArtifact, id: &str) -> &'a WeightQuantEntry {
        export
            .core
            .quant()
            .weight_quant()
            .iter()
            .find(|entry| entry.weight.as_str() == id)
            .unwrap_or_else(|| panic!("missing weight quant entry {id}"))
    }

    fn fixture_export() -> ExportedQatArtifact {
        let mut visitor = export_visitor();
        visitor
            .visit_expert_block("block.0.ffn", &fixture_expert_block())
            .unwrap();
        visitor
            .visit_router("block.0.router", &fixture_router())
            .unwrap();
        visitor.visit_norm("block.0.norm", &fixture_norm()).unwrap();
        visitor
            .visit_embedding("token_embedding", 2, 2, &[0.1, 0.2, 0.3, 0.4])
            .unwrap();
        visitor
            .visit_classifier("classifier", 2, 2, &[0.4, 0.3, 0.2, 0.1])
            .unwrap();
        visitor.finish().unwrap()
    }

    fn export_visitor() -> ExportVisitor {
        ExportVisitor::new(SequenceExportFacts::for_spec(fixture_sequence()))
    }

    fn fixture_sequence() -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::bounded_kv(16, 8).unwrap()
    }

    fn fixture_expert_block() -> ExpertBlockQat {
        ExpertBlockQat::new(vec![fixture_expert()], Some(fixture_shared_dense())).unwrap()
    }

    fn fixture_expert() -> ExpertQat {
        ExpertQat::new(
            ternary_linear(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
                None,
            ),
            activation(),
            ternary_linear(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, -1.0,
                ],
                Some(vec![0.0, 0.0]),
            ),
        )
        .unwrap()
    }

    fn fixture_gelu_expert() -> ExpertQat {
        ExpertQat::new_with_clipped_activation(
            ternary_linear(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
                None,
            ),
            ClippedActivation::gelu_clip(),
            activation(),
            ternary_linear(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, -1.0,
                ],
                Some(vec![0.0, 0.0]),
            ),
        )
        .unwrap()
    }

    fn fixture_shared_dense() -> SharedDenseBranch {
        SharedDenseBranch::new(
            DenseBranchProjection::new(
                MatrixShape::new(1, 2).unwrap(),
                vec![1.0, 1.0],
                Some(vec![0.0]),
            )
            .unwrap(),
            activation(),
            DenseBranchProjection::new(
                MatrixShape::new(2, 1).unwrap(),
                vec![1.0, 2.0],
                Some(vec![0.0, 0.0]),
            )
            .unwrap(),
            0.25,
        )
        .unwrap()
    }

    fn fixture_router() -> Top1RouterQat {
        Top1RouterQat::new(
            RouterShape::new(2, 2, 1).unwrap(),
            vec![1.0, -1.0],
            Some(vec![0.1]),
            vec![1.0, -1.0],
            Some(vec![0.0, 0.25]),
        )
        .unwrap()
    }

    fn fixture_linear_state_block() -> LinearStateBlock {
        let mut input_to_state = vec![0.0; 16 * 2];
        input_to_state[0] = 1.0;
        input_to_state[3] = 1.0;

        let mut state_to_output = vec![0.0; 2 * 16];
        state_to_output[0] = 1.0;
        state_to_output[17] = 1.0;

        LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 64).unwrap(),
            fixture_norm(),
            activation(),
            ternary_linear(16, 2, input_to_state, None),
            ternary_linear(2, 16, state_to_output, None),
            activation(),
        )
        .unwrap()
    }

    fn fixture_bounded_kv_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 32, 8).unwrap(),
            fixture_norm(),
            activation(),
            ternary_linear(1, 2, vec![0.0, 0.0], None),
            ternary_linear(1, 2, vec![1.0, 1.0], None),
            ternary_linear(2, 1, vec![1.0, -1.0], None),
            activation(),
        )
        .unwrap()
    }

    fn fixture_norm() -> NormApproxQat {
        NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(2.0, -1.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
        })
    }

    fn activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
        .with_eval_passthrough(false)
    }

    fn ema_activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Ema {
                range: ActivationRange::new(-2.0, 2.0).unwrap(),
                decay: EmaDecay::new(0.9).unwrap(),
            },
            ActivationQuantFormat::UInt4,
        )
        .unwrap()
        .with_eval_passthrough(true)
    }

    fn ternary_linear(
        output_rows: usize,
        input_cols: usize,
        weights: Vec<f32>,
        bias: Option<Vec<f32>>,
    ) -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(output_rows, input_cols).unwrap(),
            weights,
            bias,
            vec![TernaryThreshold::new(0.5).unwrap(); output_rows],
            vec![Q8_8Scale::from_f32(1.0).unwrap(); output_rows],
        )
        .unwrap()
    }
}
