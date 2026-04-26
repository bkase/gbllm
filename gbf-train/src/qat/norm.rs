//! Burn-backed normalization approximation QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{
    AffineParams, LutSpec, NormApproxError, NormApproxPlan, NormApproxQat, NormClip,
    NormExportData, TileRmsSpec,
};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam, BurnShape,
    float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape, ste_replace_forward,
};

const MIN_CLIP_HALF_WIDTH: f32 = f32::EPSILON;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormApproxBurnPlan {
    AffineClipLut { lut: LutSpec },
    TileRmsThenAffineClip { tile: TileRmsSpec },
}

#[derive(BurnModule, Debug)]
pub struct NormApproxBurnQat<B: BurnBackend> {
    #[module(skip)]
    plan: NormApproxBurnPlan,
    affine_scale: BurnParam<BurnFloatTensor<B, 1>>,
    affine_bias: BurnParam<BurnFloatTensor<B, 1>>,
    clip_center: BurnParam<BurnFloatTensor<B, 1>>,
    clip_half_width: BurnParam<BurnFloatTensor<B, 1>>,
}

impl<B: BurnBackend> NormApproxBurnQat<B> {
    pub fn from_core(
        core: NormApproxQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, NormApproxBurnQatError> {
        let (plan, affine, clip) = split_core_plan(core.plan());
        let clip_center = (clip.lo() + clip.hi()) / 2.0;
        let clip_half_width = (clip.hi() - clip.lo()) / 2.0;

        Ok(Self {
            plan,
            affine_scale: BurnParam::from_tensor(scalar_tensor(affine.scale(), device)?),
            affine_bias: BurnParam::from_tensor(scalar_tensor(affine.bias(), device)?),
            clip_center: BurnParam::from_tensor(scalar_tensor(clip_center, device)?),
            clip_half_width: BurnParam::from_tensor(scalar_tensor(clip_half_width, device)?),
        })
    }

    #[must_use]
    pub fn plan(&self) -> NormApproxBurnPlan {
        self.plan
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
    pub fn clip_center(&self) -> BurnFloatTensor<B, 1> {
        self.clip_center.val()
    }

    #[must_use]
    pub fn clip_half_width(&self) -> BurnFloatTensor<B, 1> {
        self.clip_half_width.val()
    }

    #[must_use]
    pub fn clip_lo(&self) -> BurnFloatTensor<B, 1> {
        let (lo, _) = self.scalar_clip_bounds();
        lo
    }

    #[must_use]
    pub fn clip_hi(&self) -> BurnFloatTensor<B, 1> {
        let (_, hi) = self.scalar_clip_bounds();
        hi
    }

    pub fn forward<const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
    ) -> Result<BurnFloatTensor<B, D>, NormApproxBurnQatError> {
        match self.plan {
            NormApproxBurnPlan::AffineClipLut { lut } => Ok(self.affine_clip_lut(input, lut)),
            NormApproxBurnPlan::TileRmsThenAffineClip { tile } => {
                Ok(self.affine_clip(tile_rms(input, tile)?))
            }
        }
    }

    pub fn export_norm_params(&self) -> Result<NormExportData, NormApproxBurnQatError> {
        let affine = self.current_affine()?;
        let clip = self.current_clip()?;
        let plan = match self.plan {
            NormApproxBurnPlan::AffineClipLut { lut } => {
                NormApproxPlan::AffineClipLut { affine, clip, lut }
            }
            NormApproxBurnPlan::TileRmsThenAffineClip { tile } => {
                NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip }
            }
        };

        Ok(NormApproxQat::new(plan).export_norm_params())
    }

    fn affine_clip<const D: usize>(&self, input: BurnFloatTensor<B, D>) -> BurnFloatTensor<B, D> {
        let shape = input.shape();
        let scale: BurnFloatTensor<B, D> = self.affine_scale.val().expand(shape.clone());
        let bias: BurnFloatTensor<B, D> = self.affine_bias.val().expand(shape.clone());
        let (lo, hi) = self.clip_bounds_for_shape(shape);

        (input * scale + bias).max_pair(lo).min_pair(hi)
    }

    fn affine_clip_lut<const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
        lut: LutSpec,
    ) -> BurnFloatTensor<B, D> {
        let surrogate = self.affine_clip(input.clone());
        let hard = self.hard_affine_clip_lut(input, lut);

        ste_replace_forward(surrogate, hard)
    }

    fn hard_affine_clip_lut<const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
        lut: LutSpec,
    ) -> BurnFloatTensor<B, D> {
        let shape = input.shape();
        let step = (lut.input_hi() - lut.input_lo()) / (lut.entries() - 1) as f32;
        let indices =
            ((input.clone().clamp(lut.input_lo(), lut.input_hi()) - lut.input_lo()) / step).round();
        let mut output = input.zeros_like();

        for index in 0..lut.entries() {
            let sample = lut.input_lo() + step * index as f32;
            let value = self.affine_clip_sample(sample, shape.clone());
            output = output.mask_where(indices.clone().equal_elem(index as f32), value);
        }

        output
    }

    fn affine_clip_sample<const D: usize>(
        &self,
        sample: f32,
        shape: BurnShape,
    ) -> BurnFloatTensor<B, D> {
        let sample = self.affine_scale.val().zeros_like() + sample;

        self.affine_clip(sample).expand(shape)
    }

    fn clip_bounds_for_shape<const D: usize>(
        &self,
        shape: BurnShape,
    ) -> (BurnFloatTensor<B, D>, BurnFloatTensor<B, D>) {
        let (lo, hi) = self.scalar_clip_bounds();

        (lo.expand(shape.clone()), hi.expand(shape))
    }

    fn scalar_clip_bounds(&self) -> (BurnFloatTensor<B, 1>, BurnFloatTensor<B, 1>) {
        let center = self.clip_center.val();
        let half_width = self
            .clip_half_width
            .val()
            .abs()
            .clamp_min(MIN_CLIP_HALF_WIDTH);

        (center.clone() - half_width.clone(), center + half_width)
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
    RankZeroTileInput,
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
            Self::RankZeroTileInput => f.write_str("norm tile RMS requires rank >= 1 input"),
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

fn split_core_plan(plan: NormApproxPlan) -> (NormApproxBurnPlan, AffineParams, NormClip) {
    match plan {
        NormApproxPlan::AffineClipLut { affine, clip, lut } => {
            (NormApproxBurnPlan::AffineClipLut { lut }, affine, clip)
        }
        NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip } => (
            NormApproxBurnPlan::TileRmsThenAffineClip { tile },
            affine,
            clip,
        ),
    }
}

fn tile_rms<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    tile: TileRmsSpec,
) -> Result<BurnFloatTensor<B, D>, NormApproxBurnQatError> {
    let shape = float_tensor_shape(&input);
    let len = checked_element_count(&shape)?;
    let last_dim = shape
        .last()
        .copied()
        .ok_or(NormApproxBurnQatError::RankZeroTileInput)?;
    let tile_width = tile.tile_width();
    if !last_dim.is_multiple_of(tile_width) {
        return Err(NormApproxError::RaggedTileInput {
            len: last_dim,
            tile_width,
        }
        .into());
    }

    if len == 0 {
        return Ok(input);
    }

    let original_shape = input.shape();
    let tiled: BurnFloatTensor<B, 2> = input.reshape([len / tile_width, tile_width]);
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
    fn burn_norm_affine_clip_lut_uses_lut_forward_with_surrogate_gradients() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![-2.0, 0.25, 2.0, 3.0], [4], &device)
            .unwrap()
            .require_grad();

        let output = layer.forward(input.clone()).unwrap();
        let gradients = output.clone().sum().backward();

        assert_eq!(
            float_tensor_into_vec(output.inner()).unwrap(),
            vec![-1.0, 0.0, 1.0, 1.0]
        );
        assert_eq!(
            float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap(),
            vec![0.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.affine_scale().grad(&gradients).unwrap()).unwrap(),
            vec![0.25]
        );
        assert_eq!(
            float_tensor_into_vec(layer.affine_bias().grad(&gradients).unwrap()).unwrap(),
            vec![1.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.clip_center().grad(&gradients).unwrap()).unwrap(),
            vec![3.0]
        );
        assert_eq!(
            float_tensor_into_vec(layer.clip_half_width().grad(&gradients).unwrap()).unwrap(),
            vec![1.0]
        );
    }

    #[test]
    fn burn_norm_tile_rms_has_input_affine_and_clip_gradients() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![10.0, 0.0, 1.0, 0.0], [4], &device)
            .unwrap()
            .require_grad();

        let output = layer.forward(input.clone()).unwrap();
        let gradients = output.sum().backward();
        let input_grad = float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap();

        assert!(input_grad.iter().all(|value| value.is_finite()));
        assert!(input_grad.iter().any(|value| value.abs() > 0.0));
        assert!(
            float_tensor_into_vec(layer.affine_scale().grad(&gradients).unwrap()).unwrap()[0] > 0.0
        );
        assert!(
            float_tensor_into_vec(layer.affine_bias().grad(&gradients).unwrap()).unwrap()[0] > 0.0
        );
        assert!(
            float_tensor_into_vec(layer.clip_center().grad(&gradients).unwrap()).unwrap()[0] > 0.0
        );
        assert!(
            float_tensor_into_vec(layer.clip_half_width().grad(&gradients).unwrap()).unwrap()[0]
                > 0.0
        );
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

    #[test]
    fn burn_norm_rejects_tile_rms_ragged_last_axis_even_when_total_len_is_divisible() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input =
            float_tensor_from_vec::<B, 2>(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], [2, 3], &device)
                .unwrap();

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

    #[test]
    fn burn_norm_handles_empty_tiled_batches_without_reshape_panic() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0e-5).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });
        let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![], [0, 4], &device).unwrap();

        let output = layer.forward(input).unwrap();

        assert_eq!(float_tensor_shape(&output), [0, 4]);
        assert_eq!(float_tensor_into_vec(output).unwrap(), Vec::<f32>::new());
    }

    #[test]
    fn burn_norm_export_keeps_clip_bounds_ordered_after_negative_width_step() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = NormApproxBurnQat::<B> {
            plan: NormApproxBurnPlan::AffineClipLut {
                lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
            },
            affine_scale: BurnParam::from_tensor(scalar_tensor(1.0, &device).unwrap()),
            affine_bias: BurnParam::from_tensor(scalar_tensor(0.0, &device).unwrap()),
            clip_center: BurnParam::from_tensor(scalar_tensor(0.0, &device).unwrap()),
            clip_half_width: BurnParam::from_tensor(scalar_tensor(-1.0, &device).unwrap()),
        };

        // The raw width parameter is structurally interpreted by abs()+floor,
        // so even a negative optimizer step leaves export with lo < hi.
        let export = layer.export_norm_params().unwrap();

        assert!(matches!(export.plan(), NormPlan::AffineClipLut(_)));
        assert!(
            float_tensor_into_vec(layer.clip_hi()).unwrap()[0]
                > float_tensor_into_vec(layer.clip_lo()).unwrap()[0]
        );
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
