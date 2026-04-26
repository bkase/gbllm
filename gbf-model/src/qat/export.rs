//! Deterministic export visitor for backend-independent QAT modules.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_artifact::core::{ArtifactCore, ArtifactCoreError};
use gbf_artifact::export_facts::{ExportFacts, RangeDigest, RangeDigestMode};
use gbf_artifact::ids::{ArtifactPath, ArtifactPathError};
use gbf_artifact::norm_plan::NormExportParams;
use gbf_artifact::quant::{
    ActivationQuantEntry, ActivationQuantFormatSpec, NormQuantEntry, QuantSpec, TernaryQuantEntry,
};
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorError, CanonicalTensorId, CanonicalTensorKind,
    CanonicalTensorLayout, CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
};
use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use super::{
    ActFakeQuant, ActivationQuantFormat, ActivationRangeModeKind, DenseBranchProjection,
    ExpertBlockQat, ExpertQat, NormApproxQat, SharedDenseBranch, TernaryLinearQat, Top1RouterQat,
};

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
}

#[derive(Debug, Clone, Default)]
pub struct ExportVisitor {
    tensors: Vec<CanonicalTensor>,
    ternary_weight_plans: Vec<TernaryQuantEntry>,
    activation_quant: Vec<ActivationQuantEntry>,
    norm_plans: Vec<NormQuantEntry>,
    activation_ranges: Vec<RangeDigest>,
    visited_modules: Vec<VisitedModule>,
}

impl ExportVisitor {
    pub fn new() -> Self {
        Self::default()
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

    pub fn visit_embedding(
        &mut self,
        prefix: &str,
        rows: usize,
        cols: usize,
        values: &[f32],
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.add_float_tensor(path, CanonicalTensorKind::Embedding, &[rows, cols], values)
    }

    pub fn visit_classifier(
        &mut self,
        prefix: &str,
        rows: usize,
        cols: usize,
        values: &[f32],
    ) -> Result<(), ExportVisitorError> {
        let path = artifact_path(prefix)?;
        self.add_float_tensor(path, CanonicalTensorKind::Classifier, &[rows, cols], values)
    }

    pub fn finish(mut self) -> Result<ExportedQatArtifact, ExportVisitorError> {
        self.tensors.sort_by(|left, right| left.id.cmp(&right.id));
        self.ternary_weight_plans
            .sort_by(|left, right| left.projection.cmp(&right.projection));
        self.activation_quant
            .sort_by(|left, right| left.activation.cmp(&right.activation));
        self.norm_plans
            .sort_by(|left, right| left.norm.cmp(&right.norm));
        self.activation_ranges
            .sort_by(|left, right| left.activation.cmp(&right.activation));
        self.visited_modules
            .sort_by(|left, right| left.path.cmp(&right.path).then(left.kind.cmp(&right.kind)));

        ensure_unique(
            "ternary projection",
            self.ternary_weight_plans
                .iter()
                .map(|entry| &entry.projection),
        )?;
        ensure_unique(
            "activation quant entry",
            self.activation_quant.iter().map(|entry| &entry.activation),
        )?;
        ensure_unique(
            "norm entry",
            self.norm_plans.iter().map(|entry| &entry.norm),
        )?;
        ensure_unique(
            "activation range",
            self.activation_ranges.iter().map(|range| &range.activation),
        )?;

        let quant = QuantSpec::new(
            self.ternary_weight_plans,
            self.activation_quant,
            self.norm_plans,
        );
        let core = ArtifactCore::new(self.tensors, quant)?;
        let facts = ExportFacts::new(self.activation_ranges);

        Ok(ExportedQatArtifact {
            core,
            facts,
            visited_modules: self.visited_modules,
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

    fn visit_expert_at(
        &mut self,
        path: ArtifactPath,
        expert: &ExpertQat,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Expert);
        self.visit_ternary_linear_at(path.join("up")?, expert.up_projection())?;
        self.visit_activation_at(path.join("activation")?, expert.activation())?;
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

        self.ternary_weight_plans.push(TernaryQuantEntry {
            projection: path,
            weight: weight_id,
            scale: scale_id,
            bias: bias_id,
            plan: export.plan(),
        });

        Ok(())
    }

    fn visit_activation_at(
        &mut self,
        path: ArtifactPath,
        activation: &ActFakeQuant,
    ) -> Result<(), ExportVisitorError> {
        self.mark_visited(path.clone(), VisitedModuleKind::Activation);

        let range = activation.export_range();
        let quant_format = activation_quant_format(activation.quant_format());
        self.activation_ranges.push(RangeDigest {
            activation: path.clone(),
            lo: range.lo(),
            hi: range.hi(),
            mode: range_digest_mode(activation.range_mode().kind()),
            quant_format,
        });
        self.activation_quant.push(ActivationQuantEntry {
            activation: path,
            quant_format,
        });

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
            NormExportParams::None | NormExportParams::TileRmsThenAffineClip { .. } => None,
        };

        self.norm_plans.push(NormQuantEntry {
            norm: path,
            plan: export.plan(),
            lut,
        });

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
        self.add_float_tensor(
            input_projection.join("weight")?,
            CanonicalTensorKind::RouterWeight,
            &[shape.rank(), shape.d_model()],
            router.input_projection(),
        )?;
        if let Some(bias) = router.input_bias() {
            self.add_float_tensor(
                input_projection.join("bias")?,
                CanonicalTensorKind::RouterBias,
                &[shape.rank()],
                bias,
            )?;
        }

        let expert_projection = path.join("expert_projection")?;
        self.add_float_tensor(
            expert_projection.join("weight")?,
            CanonicalTensorKind::RouterWeight,
            &[shape.n_experts(), shape.rank()],
            router.expert_projection(),
        )?;
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
        self.add_float_tensor(
            path.join("weight")?,
            CanonicalTensorKind::DenseWeight,
            &[shape.output_rows(), shape.input_cols()],
            projection.weights(),
        )?;
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
        self.tensors
            .push(CanonicalTensor::new(id, kind, layout, payload)?);
        Ok(())
    }

    fn mark_visited(&mut self, path: ArtifactPath, kind: VisitedModuleKind) {
        self.visited_modules.push(VisitedModule { path, kind });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportVisitorError {
    ArtifactPath(ArtifactPathError),
    Tensor(CanonicalTensorError),
    ArtifactCore(ArtifactCoreError),
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

fn artifact_path(value: &str) -> Result<ArtifactPath, ExportVisitorError> {
    Ok(ArtifactPath::new(value)?)
}

fn ensure_unique<'a>(
    kind: &'static str,
    paths: impl Iterator<Item = &'a ArtifactPath>,
) -> Result<(), ExportVisitorError> {
    let mut seen = BTreeSet::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            return Err(ExportVisitorError::DuplicatePath {
                kind,
                path: path.clone(),
            });
        }
    }

    Ok(())
}

fn activation_quant_format(format: ActivationQuantFormat) -> ActivationQuantFormatSpec {
    match format {
        ActivationQuantFormat::Int8 => ActivationQuantFormatSpec::Int8,
        ActivationQuantFormat::UInt8 => ActivationQuantFormatSpec::UInt8,
        ActivationQuantFormat::UInt4 => ActivationQuantFormatSpec::UInt4,
    }
}

fn range_digest_mode(kind: ActivationRangeModeKind) -> RangeDigestMode {
    match kind {
        ActivationRangeModeKind::Fixed => RangeDigestMode::Fixed,
        ActivationRangeModeKind::Learned => RangeDigestMode::Learned,
        ActivationRangeModeKind::Ema => RangeDigestMode::Ema,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_artifact::tensor::{CanonicalTensorKind, CanonicalTensorPayload};

    use super::*;
    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
        DenseBranchProjection, EmaDecay, LutSpec, MatrixShape, NormApproxPlan, NormClip, Q8_8Scale,
        RouterShape, TernaryThreshold,
    };

    #[test]
    fn qat_export_visitor_builds_deterministic_artifact_for_same_modules() {
        let block_a = fixture_expert_block();
        let block_b = fixture_expert_block();
        let router_a = fixture_router();
        let router_b = fixture_router();
        let norm_a = fixture_norm();
        let norm_b = fixture_norm();

        let mut first = ExportVisitor::new();
        first.visit_expert_block("block.0.ffn", &block_a).unwrap();
        first.visit_router("block.0.router", &router_a).unwrap();
        first.visit_norm("block.0.norm", &norm_a).unwrap();
        first
            .visit_embedding("token_embedding", 2, 2, &[0.1, 0.2, 0.3, 0.4])
            .unwrap();
        first
            .visit_classifier("classifier", 2, 2, &[0.4, 0.3, 0.2, 0.1])
            .unwrap();
        let first = first.finish().unwrap();

        let mut second = ExportVisitor::new();
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
    fn qat_export_visitor_records_all_qat_module_kinds_and_activation_ranges() {
        let block = fixture_expert_block();
        let router = fixture_router();
        let norm = fixture_norm();

        let mut visitor = ExportVisitor::new();
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
            export.facts.activation_ranges.last().unwrap().mode,
            RangeDigestMode::Ema
        ));
    }

    #[test]
    fn qat_export_visitor_emits_scale_tensors_separately_from_ternary_weights() {
        let layer = ternary_linear(2, 2, vec![1.0, -1.0, 0.25, 0.75], Some(vec![0.1, -0.1]));

        let mut visitor = ExportVisitor::new();
        visitor.visit_ternary_linear("projection", &layer).unwrap();
        let export = visitor.finish().unwrap();

        let weight = tensor(&export, "projection.weight");
        let scale = tensor(&export, "projection.scale");
        assert_eq!(weight.kind, CanonicalTensorKind::TernaryWeight);
        assert_eq!(scale.kind, CanonicalTensorKind::TernaryScale);
        assert_eq!(weight.layout.shape.dims(), &[2, 2]);
        assert_eq!(scale.layout.shape.dims(), &[2]);
        assert_eq!(
            scale.payload.as_u16_slice(),
            Some(
                &[
                    Q8_8Scale::from_f32(1.0).unwrap().raw(),
                    Q8_8Scale::from_f32(1.0).unwrap().raw(),
                ][..]
            )
        );

        let quant = &export.core.quant.ternary_weight_plans[0];
        assert_eq!(quant.weight.as_str(), "projection.weight");
        assert_eq!(quant.scale.as_str(), "projection.scale");
        assert_eq!(quant.bias.as_ref().unwrap().as_str(), "projection.bias");
    }

    #[test]
    fn qat_export_visitor_extracts_router_shared_dense_and_norm_lut_tensors() {
        let block = fixture_expert_block();
        let router = fixture_router();
        let norm = fixture_norm();

        let mut visitor = ExportVisitor::new();
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
    }

    #[test]
    fn qat_export_visitor_rejects_duplicate_paths() {
        let activation = activation();
        let mut visitor = ExportVisitor::new();
        visitor
            .visit_activation("dup.activation", &activation)
            .unwrap();
        visitor
            .visit_activation("dup.activation", &activation)
            .unwrap();

        assert_eq!(
            visitor.finish(),
            Err(ExportVisitorError::DuplicatePath {
                kind: "activation quant entry",
                path: ArtifactPath::new("dup.activation").unwrap()
            })
        );
    }

    #[test]
    fn qat_export_visitor_rejects_non_finite_raw_float_tensors() {
        let mut visitor = ExportVisitor::new();

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
            .tensors
            .iter()
            .find(|tensor| tensor.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing tensor {id}"))
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
