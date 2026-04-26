//! Burn-backed normalization approximation QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{
    AffineParams, NormApproxError, NormApproxPlan, NormApproxQat, NormClip, NormExportData,
    TileRmsSpec,
};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam,
    float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape,
};

#[derive(BurnModule, Debug)]
pub struct NormApproxBurnQat<B: BurnBackend> {
    affine_scale: BurnParam<BurnFloatTensor<B, 1>>,
    affine_bias: BurnParam<BurnFloatTensor<B, 1>>,
    clip_lo: BurnParam<BurnFloatTensor<B, 1>>,
    clip_hi: BurnParam<BurnFloatTensor<B, 1>>,
    #[module(skip)]
    core: NormApproxQat,
}

impl<B: BurnBackend> NormApproxBurnQat<B> {
    pub fn from_core(
        core: NormApproxQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, NormApproxBurnQatError> {
        let (affine, clip) = plan_affine_and_clip(core.plan());

        Ok(Self {
            affine_scale: BurnParam::from_tensor(scalar_tensor(affine.scale(), device)?),
            affine_bias: BurnParam::from_tensor(scalar_tensor(affine.bias(), device)?),
            clip_lo: BurnParam::from_tensor(scalar_tensor(clip.lo(), device)?),
            clip_hi: BurnParam::from_tensor(scalar_tensor(clip.hi(), device)?),
            core,
        })
    }

    #[must_use]
    pub fn core(&self) -> &NormApproxQat {
        &self.core
    }

    #[must_use]
    pub fn affine_scale(&self) -> BurnFloatTensor<B, 1> {
        self.affine_scale.val()
    }

    #[must_use]
    pub fn affine_bias(&self) -> BurnFloatTensor<B, 1> {
        self.affine_bias.val()
    }

    #[must_use]
    pub fn clip_lo(&self) -> BurnFloatTensor<B, 1> {
        self.clip_lo.val()
    }

    #[must_use]
    pub fn clip_hi(&self) -> BurnFloatTensor<B, 1> {
        self.clip_hi.val()
    }

    pub fn forward<const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
    ) -> Result<BurnFloatTensor<B, D>, NormApproxBurnQatError> {
        match self.core.plan() {
            NormApproxPlan::AffineClipLut { .. } => Ok(self.affine_clip(input)),
            NormApproxPlan::TileRmsThenAffineClip { tile, .. } => {
                Ok(self.affine_clip(tile_rms(input, tile)?))
            }
        }
    }

    pub fn export_norm_params(&self) -> Result<NormExportData, NormApproxBurnQatError> {
        let affine = self.current_affine()?;
        let clip = self.current_clip()?;
        let plan = match self.core.plan() {
            NormApproxPlan::AffineClipLut { lut, .. } => {
                NormApproxPlan::AffineClipLut { affine, clip, lut }
            }
            NormApproxPlan::TileRmsThenAffineClip { tile, .. } => {
                NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip }
            }
        };

        Ok(NormApproxQat::new(plan).export_norm_params())
    }

    fn affine_clip<const D: usize>(&self, input: BurnFloatTensor<B, D>) -> BurnFloatTensor<B, D> {
        let shape = input.shape();
        let scale: BurnFloatTensor<B, D> = self.affine_scale.val().expand(shape.clone());
        let bias: BurnFloatTensor<B, D> = self.affine_bias.val().expand(shape.clone());
        let lo: BurnFloatTensor<B, D> = self.clip_lo.val().expand(shape.clone());
        let hi: BurnFloatTensor<B, D> = self.clip_hi.val().expand(shape);

        (input * scale + bias).max_pair(lo).min_pair(hi)
    }

    fn current_affine(&self) -> Result<AffineParams, NormApproxBurnQatError> {
        AffineParams::new(
            scalar_from_tensor("affine_scale", self.affine_scale().detach())?,
            scalar_from_tensor("affine_bias", self.affine_bias().detach())?,
        )
        .map_err(NormApproxBurnQatError::Model)
    }

    fn current_clip(&self) -> Result<NormClip, NormApproxBurnQatError> {
        NormClip::new(
            scalar_from_tensor("clip_lo", self.clip_lo().detach())?,
            scalar_from_tensor("clip_hi", self.clip_hi().detach())?,
        )
        .map_err(NormApproxBurnQatError::Model)
    }
}

#[derive(Debug)]
pub enum NormApproxBurnQatError {
    Adapter(BurnAdapterError),
    Model(NormApproxError),
    ScalarParamLen { name: &'static str, actual: usize },
    ShapeElementCountOverflow,
}

impl fmt::Display for NormApproxBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::ScalarParamLen { name, actual } => {
                write!(
                    f,
                    "norm scalar parameter {name} expected length 1, got {actual}"
                )
            }
            Self::ShapeElementCountOverflow => {
                f.write_str("norm input shape element count overflowed usize")
            }
        }
    }
}

impl Error for NormApproxBurnQatError {}

impl From<BurnAdapterError> for NormApproxBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<NormApproxError> for NormApproxBurnQatError {
    fn from(error: NormApproxError) -> Self {
        Self::Model(error)
    }
}

fn scalar_tensor<B: BurnBackend>(
    value: f32,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BurnAdapterError> {
    float_tensor_from_vec(vec![value], [1], device)
}

fn scalar_from_tensor<B: BurnBackend>(
    name: &'static str,
    tensor: BurnFloatTensor<B, 1>,
) -> Result<f32, NormApproxBurnQatError> {
    let values = float_tensor_into_vec(tensor)?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(NormApproxBurnQatError::ScalarParamLen {
            name,
            actual: values.len(),
        }),
    }
}

fn plan_affine_and_clip(plan: NormApproxPlan) -> (AffineParams, NormClip) {
    match plan {
        NormApproxPlan::AffineClipLut { affine, clip, .. }
        | NormApproxPlan::TileRmsThenAffineClip { affine, clip, .. } => (affine, clip),
    }
}

fn tile_rms<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    tile: TileRmsSpec,
) -> Result<BurnFloatTensor<B, D>, NormApproxBurnQatError> {
    let shape = float_tensor_shape(&input);
    let len = checked_element_count(&shape)?;
    let tile_width = tile.tile_width();
    if len != 0 && !len.is_multiple_of(tile_width) {
        return Err(NormApproxError::RaggedTileInput { len, tile_width }.into());
    }

    let original_shape = input.shape();
    let tiled: BurnFloatTensor<B, 2> = input.reshape([-1, tile_width as i32]);
    let mean_square = (tiled.clone() * tiled.clone()).mean_dim(1);
    let rms = (mean_square + tile.epsilon())
        .sqrt()
        .repeat_dim(1, tile_width);

    Ok((tiled / rms).reshape(original_shape))
}

fn checked_element_count<const D: usize>(
    shape: &[usize; D],
) -> Result<usize, NormApproxBurnQatError> {
    shape
        .iter()
        .try_fold(1usize, |acc, dim| acc.checked_mul(*dim))
        .ok_or(NormApproxBurnQatError::ShapeElementCountOverflow)
}

#[cfg(test)]
mod tests {
    use gbf_artifact::norm_plan::{NormExportParams as ArtifactNormExportParams, NormPlan};
    use gbf_model::qat::{LutSpec, NormClip};

    use super::*;
    use crate::adapter::burn::{
        BurnNdArrayAutodiffBackend, BurnNdArrayBackend, float_tensor_from_vec,
        float_tensor_into_vec, float_tensor_shape,
    };

    #[test]
    fn burn_norm_affine_clip_lut_forward_matches_scalar_core() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(2.0, -1.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![-1.0, 0.0, 0.75];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [3], &device).unwrap();

        let output = layer.forward(tensor).unwrap();

        assert_eq!(
            float_tensor_into_vec(output).unwrap(),
            core.forward(&input).unwrap()
        );
    }

    #[test]
    fn burn_norm_tile_rms_forward_matches_scalar_core_and_preserves_shape() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![1.0, 1.0, 0.0, 2.0];
        let tensor = float_tensor_from_vec::<B, 2>(input.clone(), [2, 2], &device).unwrap();

        let output = layer.forward(tensor).unwrap();

        assert_eq!(float_tensor_shape(&output), [2, 2]);
        assert_close(
            &float_tensor_into_vec(output).unwrap(),
            &core.forward(&input).unwrap(),
            1.0e-6,
        );
    }

    #[test]
    fn burn_norm_affine_clip_lut_gradients_reach_input_affine_and_clip_bounds() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![-2.0, 0.5, 2.0], [3], &device)
            .unwrap()
            .require_grad();

        let output = layer.forward(input.clone()).unwrap();
        let gradients = output.clone().sum().backward();

        assert_eq!(
            float_tensor_into_vec(output.inner()).unwrap(),
            vec![-1.0, 0.5, 1.0]
        );
        assert_eq!(
            float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap(),
            vec![0.0, 1.0, 0.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.affine_scale().grad(&gradients).unwrap()).unwrap(),
            vec![0.5]
        );
        assert_eq!(
            float_tensor_into_vec(layer.affine_bias().grad(&gradients).unwrap()).unwrap(),
            vec![1.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.clip_lo().grad(&gradients).unwrap()).unwrap(),
            vec![1.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.clip_hi().grad(&gradients).unwrap()).unwrap(),
            vec![1.0]
        );
    }

    #[test]
    fn burn_norm_tile_rms_has_input_gradient() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0], [2], &device)
            .unwrap()
            .require_grad();

        let output = layer.forward(input.clone()).unwrap();
        let gradients = output.sum().backward();
        let input_grad = float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap();

        assert!(input_grad.iter().all(|value| value.is_finite()));
        assert!(input_grad.iter().any(|value| value.abs() > 0.0));
    }

    #[test]
    fn burn_norm_export_matches_artifact_contract_from_owned_tensors() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let affine = AffineParams::new(2.0, -1.0).unwrap();
        let clip = NormClip::new(-1.0, 1.0).unwrap();
        let lut = LutSpec::new(-1.0, 1.0, 3).unwrap();
        let core = NormApproxQat::new(NormApproxPlan::AffineClipLut { affine, clip, lut });
        let layer = NormApproxBurnQat::<B>::from_core(core.clone(), &device).unwrap();

        let export = layer.export_norm_params().unwrap();

        assert_eq!(export, core.export_norm_params());
        assert!(matches!(export.plan(), NormPlan::AffineClipLut(_)));
        assert!(matches!(
            export.params(),
            ArtifactNormExportParams::AffineClipLut { lut_values, .. }
                if lut_values == &vec![-1.0, -1.0, 1.0]
        ));
    }

    #[test]
    fn burn_norm_rejects_ragged_tile_input_before_reshape() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, 3.0], [3], &device).unwrap();

        assert!(matches!(
            layer.forward(input),
            Err(NormApproxBurnQatError::Model(
                NormApproxError::RaggedTileInput {
                    len: 3,
                    tile_width: 2,
                }
            ))
        ));
    }

    fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - expected).abs() <= tolerance,
                "{actual} != {expected} within {tolerance}"
            );
        }
    }
}
