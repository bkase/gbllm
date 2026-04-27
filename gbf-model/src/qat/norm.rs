//! Backend-independent normalization approximation core.

use std::error::Error;
use std::fmt;

use gbf_artifact::norm_plan::{
    AffineClipLutPlan as ArtifactAffineClipLutPlan, NormAffineParams as ArtifactNormAffineParams,
    NormClipBounds as ArtifactNormClipBounds, NormExportParams as ArtifactNormExportParams,
    NormLutSpec as ArtifactNormLutSpec, NormPlan as ArtifactNormPlan,
    NormTileRmsSpec as ArtifactNormTileRmsSpec,
    TileRmsThenAffineClipPlan as ArtifactTileRmsThenAffineClipPlan,
};

use super::{DEFAULT_SOFT_BLEND, QatHardnessControl, QuantHardness};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineParams {
    scale: f32,
    bias: f32,
}

impl AffineParams {
    pub fn new(scale: f32, bias: f32) -> Result<Self, NormApproxError> {
        if !scale.is_finite() {
            return Err(NormApproxError::InvalidAffineParam {
                name: "scale",
                value: scale,
            });
        }

        if !bias.is_finite() {
            return Err(NormApproxError::InvalidAffineParam {
                name: "bias",
                value: bias,
            });
        }

        Ok(Self { scale, bias })
    }

    pub fn scale(self) -> f32 {
        self.scale
    }

    pub fn bias(self) -> f32 {
        self.bias
    }

    fn apply(self, value: f32) -> f32 {
        value * self.scale + self.bias
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormClip {
    lo: f32,
    hi: f32,
}

impl NormClip {
    pub fn new(lo: f32, hi: f32) -> Result<Self, NormApproxError> {
        if !lo.is_finite() {
            return Err(NormApproxError::InvalidClipBound {
                name: "lo",
                value: lo,
            });
        }

        if !hi.is_finite() {
            return Err(NormApproxError::InvalidClipBound {
                name: "hi",
                value: hi,
            });
        }

        if lo >= hi {
            return Err(NormApproxError::InvalidClipOrder { lo, hi });
        }

        Ok(Self { lo, hi })
    }

    pub fn lo(self) -> f32 {
        self.lo
    }

    pub fn hi(self) -> f32 {
        self.hi
    }

    fn apply(self, value: f32) -> f32 {
        value.clamp(self.lo, self.hi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LutSpec {
    // Pre-affine input sampling domain for the deployable lookup table.
    // These bounds are intentionally separate from the output clip bounds.
    input_lo: f32,
    input_hi: f32,
    entries: usize,
}

impl LutSpec {
    pub fn new(input_lo: f32, input_hi: f32, entries: usize) -> Result<Self, NormApproxError> {
        if !input_lo.is_finite() {
            return Err(NormApproxError::InvalidLutBound {
                name: "input_lo",
                value: input_lo,
            });
        }

        if !input_hi.is_finite() {
            return Err(NormApproxError::InvalidLutBound {
                name: "input_hi",
                value: input_hi,
            });
        }

        if input_lo >= input_hi {
            return Err(NormApproxError::InvalidLutOrder { input_lo, input_hi });
        }

        if entries < 2 || entries > usize::from(u16::MAX) {
            return Err(NormApproxError::InvalidLutEntries(entries));
        }

        Ok(Self {
            input_lo,
            input_hi,
            entries,
        })
    }

    pub fn input_lo(self) -> f32 {
        self.input_lo
    }

    pub fn input_hi(self) -> f32 {
        self.input_hi
    }

    pub fn entries(self) -> usize {
        self.entries
    }

    pub fn nearest_index(self, value: f32) -> usize {
        if value <= self.input_lo {
            return 0;
        }

        if value >= self.input_hi {
            return self.entries - 1;
        }

        let step = (self.input_hi - self.input_lo) / (self.entries - 1) as f32;
        ((value - self.input_lo) / step).round() as usize
    }

    fn sample(self, index: usize) -> f32 {
        let denom = (self.entries - 1) as f32;
        self.input_lo + (self.input_hi - self.input_lo) * index as f32 / denom
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRmsSpec {
    tile_width: usize,
    epsilon: f32,
}

impl TileRmsSpec {
    pub fn new(tile_width: usize, epsilon: f32) -> Result<Self, NormApproxError> {
        if tile_width == 0 || tile_width > usize::from(u16::MAX) {
            return Err(NormApproxError::InvalidTileWidth(tile_width));
        }

        if !epsilon.is_finite() || epsilon <= 0.0 {
            return Err(NormApproxError::InvalidEpsilon(epsilon));
        }

        Ok(Self {
            tile_width,
            epsilon,
        })
    }

    pub fn tile_width(self) -> usize {
        self.tile_width
    }

    pub fn epsilon(self) -> f32 {
        self.epsilon
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormApproxPlan {
    AffineClipLut {
        affine: AffineParams,
        clip: NormClip,
        lut: LutSpec,
    },
    TileRmsThenAffineClip {
        tile: TileRmsSpec,
        affine: AffineParams,
        clip: NormClip,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormExportData {
    params: ArtifactNormExportParams,
}

impl NormExportData {
    pub fn params(&self) -> &ArtifactNormExportParams {
        &self.params
    }

    pub fn plan(&self) -> ArtifactNormPlan {
        match &self.params {
            ArtifactNormExportParams::AffineClipLut { plan, .. } => {
                ArtifactNormPlan::AffineClipLut(*plan)
            }
            ArtifactNormExportParams::TileRmsThenAffineClip { plan } => {
                ArtifactNormPlan::TileRmsThenAffineClip(*plan)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormApproxQat {
    plan: NormApproxPlan,
    hardness: QuantHardness,
}

impl NormApproxQat {
    pub fn new(plan: NormApproxPlan) -> Self {
        Self {
            plan,
            hardness: QuantHardness::Hard,
        }
    }

    pub fn plan(&self) -> NormApproxPlan {
        self.plan
    }

    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>, NormApproxError> {
        validate_input(input)?;

        match self.plan {
            NormApproxPlan::AffineClipLut { affine, clip, lut } => {
                let forward = match self.hardness {
                    QuantHardness::Off => exact_affine_clip_lut,
                    QuantHardness::Soft => soft_affine_clip_lut,
                    QuantHardness::Hard => apply_affine_clip_lut,
                };
                Ok(input
                    .iter()
                    .copied()
                    .map(|value| forward(value, affine, clip, lut))
                    .collect())
            }
            NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip } => match self.hardness {
                QuantHardness::Off => full_rms_then_affine_clip(input, affine, clip),
                QuantHardness::Soft => {
                    let exact = full_rms_then_affine_clip(input, affine, clip)?;
                    let hard = tile_rms_then_affine_clip(input, tile, affine, clip)?;
                    Ok(blend_vectors(&exact, &hard, DEFAULT_SOFT_BLEND))
                }
                QuantHardness::Hard => tile_rms_then_affine_clip(input, tile, affine, clip),
            },
        }
    }

    pub fn export_norm_params(&self) -> NormExportData {
        let params = match self.plan {
            NormApproxPlan::AffineClipLut { affine, clip, lut } => {
                ArtifactNormExportParams::AffineClipLut {
                    plan: ArtifactAffineClipLutPlan {
                        affine: artifact_affine(affine),
                        clip: artifact_clip(clip),
                        lut: artifact_lut(lut),
                    },
                    lut_values: affine_clip_lut_values(affine, clip, lut),
                }
            }
            NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip } => {
                ArtifactNormExportParams::TileRmsThenAffineClip {
                    plan: ArtifactTileRmsThenAffineClipPlan {
                        tile: artifact_tile(tile),
                        affine: artifact_affine(affine),
                        clip: artifact_clip(clip),
                    },
                }
            }
        };

        NormExportData { params }
    }
}

impl QatHardnessControl for NormApproxQat {
    fn hardness(&self) -> QuantHardness {
        self.hardness
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        self.hardness = hardness;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NormApproxError {
    InvalidAffineParam { name: &'static str, value: f32 },
    InvalidClipBound { name: &'static str, value: f32 },
    InvalidClipOrder { lo: f32, hi: f32 },
    InvalidLutBound { name: &'static str, value: f32 },
    InvalidLutOrder { input_lo: f32, input_hi: f32 },
    InvalidLutEntries(usize),
    InvalidTileWidth(usize),
    InvalidEpsilon(f32),
    RaggedTileInput { len: usize, tile_width: usize },
    NonFiniteInput { index: usize },
}

impl fmt::Display for NormApproxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAffineParam { name, value } => {
                write!(f, "norm affine parameter {name} is not finite: {value}")
            }
            Self::InvalidClipBound { name, value } => {
                write!(f, "norm clip bound {name} is not finite: {value}")
            }
            Self::InvalidClipOrder { lo, hi } => {
                write!(f, "norm clip range must satisfy lo < hi, got {lo} >= {hi}")
            }
            Self::InvalidLutBound { name, value } => {
                write!(f, "norm LUT bound {name} is not finite: {value}")
            }
            Self::InvalidLutOrder { input_lo, input_hi } => write!(
                f,
                "norm LUT input range must satisfy input_lo < input_hi, got {input_lo} >= {input_hi}"
            ),
            Self::InvalidLutEntries(entries) => {
                write!(
                    f,
                    "norm LUT entries must be in [2, {}], got {entries}",
                    u16::MAX
                )
            }
            Self::InvalidTileWidth(tile_width) => {
                write!(
                    f,
                    "norm tile width must be in [1, {}], got {tile_width}",
                    u16::MAX
                )
            }
            Self::InvalidEpsilon(epsilon) => {
                write!(f, "norm epsilon must be finite and positive, got {epsilon}")
            }
            Self::RaggedTileInput { len, tile_width } => {
                write!(
                    f,
                    "norm input length {len} is not divisible by tile width {tile_width}"
                )
            }
            Self::NonFiniteInput { index } => {
                write!(f, "norm input at index {index} is not finite")
            }
        }
    }
}

impl Error for NormApproxError {}

fn validate_input(input: &[f32]) -> Result<(), NormApproxError> {
    if let Some(index) = input.iter().position(|value| !value.is_finite()) {
        return Err(NormApproxError::NonFiniteInput { index });
    }

    Ok(())
}

fn apply_affine_clip(value: f32, affine: AffineParams, clip: NormClip) -> f32 {
    clip.apply(affine.apply(value))
}

fn exact_affine_clip_lut(value: f32, affine: AffineParams, clip: NormClip, _lut: LutSpec) -> f32 {
    apply_affine_clip(value, affine, clip)
}

fn affine_clip_lut_values(affine: AffineParams, clip: NormClip, lut: LutSpec) -> Vec<f32> {
    (0..lut.entries())
        .map(|index| apply_affine_clip(lut.sample(index), affine, clip))
        .collect()
}

fn apply_affine_clip_lut(value: f32, affine: AffineParams, clip: NormClip, lut: LutSpec) -> f32 {
    let values = affine_clip_lut_values(affine, clip, lut);
    values[lut.nearest_index(value)]
}

fn soft_affine_clip_lut(value: f32, affine: AffineParams, clip: NormClip, lut: LutSpec) -> f32 {
    apply_affine_clip(value, affine, clip) * DEFAULT_SOFT_BLEND
        + apply_affine_clip_lut(value, affine, clip, lut) * (1.0 - DEFAULT_SOFT_BLEND)
}

fn artifact_affine(affine: AffineParams) -> ArtifactNormAffineParams {
    ArtifactNormAffineParams {
        scale: affine.scale(),
        bias: affine.bias(),
    }
}

fn artifact_clip(clip: NormClip) -> ArtifactNormClipBounds {
    ArtifactNormClipBounds {
        lo: clip.lo(),
        hi: clip.hi(),
    }
}

fn artifact_lut(lut: LutSpec) -> ArtifactNormLutSpec {
    ArtifactNormLutSpec {
        input_lo: lut.input_lo(),
        input_hi: lut.input_hi(),
        entries: lut.entries() as u16,
    }
}

fn artifact_tile(tile: TileRmsSpec) -> ArtifactNormTileRmsSpec {
    ArtifactNormTileRmsSpec {
        tile_width: tile.tile_width() as u16,
        epsilon: tile.epsilon(),
    }
}

fn tile_rms_then_affine_clip(
    input: &[f32],
    tile: TileRmsSpec,
    affine: AffineParams,
    clip: NormClip,
) -> Result<Vec<f32>, NormApproxError> {
    if !input.is_empty() && !input.len().is_multiple_of(tile.tile_width()) {
        return Err(NormApproxError::RaggedTileInput {
            len: input.len(),
            tile_width: tile.tile_width(),
        });
    }

    Ok(input
        .chunks(tile.tile_width())
        .flat_map(|chunk| {
            let mean_square =
                chunk.iter().map(|value| value * value).sum::<f32>() / chunk.len() as f32;
            let rms = (mean_square + tile.epsilon()).sqrt();
            chunk
                .iter()
                .copied()
                .map(move |value| apply_affine_clip(value / rms, affine, clip))
        })
        .collect())
}

fn full_rms_then_affine_clip(
    input: &[f32],
    affine: AffineParams,
    clip: NormClip,
) -> Result<Vec<f32>, NormApproxError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mean_square = input.iter().map(|value| value * value).sum::<f32>() / input.len() as f32;
    let rms = mean_square.sqrt().max(f32::MIN_POSITIVE);

    Ok(input
        .iter()
        .copied()
        .map(|value| apply_affine_clip(value / rms, affine, clip))
        .collect())
}

fn blend_vectors(exact: &[f32], hard: &[f32], exact_weight: f32) -> Vec<f32> {
    exact
        .iter()
        .copied()
        .zip(hard.iter().copied())
        .map(|(exact, hard)| exact * exact_weight + hard * (1.0 - exact_weight))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qat_norm_affine_clip_lut_forward_and_export_match() {
        let norm = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(2.0, -1.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
        });

        let export = norm.export_norm_params();
        assert!(matches!(export.plan(), ArtifactNormPlan::AffineClipLut(_)));
        assert!(matches!(
            export.params(),
            ArtifactNormExportParams::AffineClipLut { lut_values, .. }
                if lut_values == &vec![-1.0, -1.0, 1.0]
        ));
        assert_eq!(
            norm.forward(&[-1.0, 0.0, 0.75]).unwrap(),
            vec![-1.0, -1.0, 1.0]
        );
    }

    #[test]
    fn qat_norm_affine_clip_lut_forward_uses_nearest_exported_entry() {
        let norm = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 5).unwrap(),
        });

        assert_eq!(
            norm.forward(&[-2.0, -0.76, -0.74, -0.26, 0.26, 1.2])
                .unwrap(),
            vec![-1.0, -1.0, -0.5, -0.5, 0.5, 1.0]
        );
    }

    #[test]
    fn qat_norm_tile_rms_then_affine_clip_uses_per_tile_rms() {
        let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        let output = norm.forward(&[1.0, 1.0, 0.0, 2.0]).unwrap();

        assert_close(output[0], 1.0 / 2.0_f32.sqrt());
        assert_close(output[1], 1.0 / 2.0_f32.sqrt());
        assert_eq!(output[2], 0.0);
        assert_close(output[3], 2.0 / 3.0_f32.sqrt());
        assert!(matches!(
            norm.export_norm_params().plan(),
            ArtifactNormPlan::TileRmsThenAffineClip(_)
        ));
        assert!(matches!(
            norm.export_norm_params().params(),
            ArtifactNormExportParams::TileRmsThenAffineClip { .. }
        ));
    }

    #[test]
    fn qat_norm_rejects_invalid_contracts() {
        assert!(AffineParams::new(f32::NAN, 0.0).is_err());
        assert!(AffineParams::new(1.0, f32::INFINITY).is_err());
        assert!(NormClip::new(1.0, 1.0).is_err());
        assert!(NormClip::new(f32::NAN, 1.0).is_err());
        assert!(LutSpec::new(-1.0, 1.0, 1).is_err());
        assert!(LutSpec::new(1.0, -1.0, 2).is_err());
        assert!(TileRmsSpec::new(0, 0.0).is_err());
        assert!(TileRmsSpec::new(1, 0.0).is_err());
        assert!(TileRmsSpec::new(1, -0.1).is_err());
    }

    #[test]
    fn qat_norm_rejects_non_finite_input() {
        let norm = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: LutSpec::new(-1.0, 1.0, 2).unwrap(),
        });

        assert_eq!(
            norm.forward(&[0.0, f32::NAN]),
            Err(NormApproxError::NonFiniteInput { index: 1 })
        );
    }

    #[test]
    fn qat_norm_rejects_ragged_tile_input() {
        let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        assert_eq!(
            norm.forward(&[1.0, 2.0, 3.0]),
            Err(NormApproxError::RaggedTileInput {
                len: 3,
                tile_width: 2,
            })
        );
    }

    #[test]
    fn qat_norm_zero_tile_with_positive_epsilon_stays_finite() {
        let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0e-5).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        assert_eq!(norm.forward(&[0.0, 0.0]).unwrap(), vec![0.0, 0.0]);
    }

    #[test]
    fn qat_norm_tile_rms_tiny_positive_epsilon_stays_finite() {
        let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0e-12).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        let output = norm.forward(&[1.0e-6, -1.0e-6]).unwrap();

        assert!(output.iter().all(|value| value.is_finite()));
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "actual={actual} expected={expected}"
        );
    }
}
