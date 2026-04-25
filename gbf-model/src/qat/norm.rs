//! Backend-independent normalization approximation core.

use std::error::Error;
use std::fmt;

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

        if entries < 2 {
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
        if tile_width == 0 {
            return Err(NormApproxError::InvalidTileWidth(tile_width));
        }

        if !epsilon.is_finite() || epsilon < 0.0 {
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
    plan: NormApproxPlan,
    lut_values: Option<Vec<f32>>,
}

impl NormExportData {
    pub fn plan(&self) -> NormApproxPlan {
        self.plan
    }

    pub fn lut_values(&self) -> Option<&[f32]> {
        self.lut_values.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormApproxQat {
    plan: NormApproxPlan,
}

impl NormApproxQat {
    pub fn new(plan: NormApproxPlan) -> Self {
        Self { plan }
    }

    pub fn plan(&self) -> NormApproxPlan {
        self.plan
    }

    pub fn forward(&self, input: &[f32]) -> Result<Vec<f32>, NormApproxError> {
        validate_input(input)?;

        match self.plan {
            NormApproxPlan::AffineClipLut { affine, clip, .. } => Ok(input
                .iter()
                .copied()
                .map(|value| apply_affine_clip(value, affine, clip))
                .collect()),
            NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip } => {
                Ok(tile_rms_then_affine_clip(input, tile, affine, clip))
            }
        }
    }

    pub fn export_norm_params(&self) -> NormExportData {
        let lut_values = match self.plan {
            NormApproxPlan::AffineClipLut { affine, clip, lut } => Some(
                (0..lut.entries())
                    .map(|index| apply_affine_clip(lut.sample(index), affine, clip))
                    .collect(),
            ),
            NormApproxPlan::TileRmsThenAffineClip { .. } => None,
        };

        NormExportData {
            plan: self.plan,
            lut_values,
        }
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
                write!(f, "norm LUT must have at least 2 entries, got {entries}")
            }
            Self::InvalidTileWidth(tile_width) => {
                write!(f, "norm tile width must be non-zero, got {tile_width}")
            }
            Self::InvalidEpsilon(epsilon) => {
                write!(
                    f,
                    "norm epsilon must be finite and non-negative, got {epsilon}"
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

fn tile_rms_then_affine_clip(
    input: &[f32],
    tile: TileRmsSpec,
    affine: AffineParams,
    clip: NormClip,
) -> Vec<f32> {
    input
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

        assert_eq!(
            norm.forward(&[-1.0, 0.0, 0.75]).unwrap(),
            vec![-1.0, -1.0, 0.5]
        );

        let export = norm.export_norm_params();
        assert!(matches!(
            export.plan(),
            NormApproxPlan::AffineClipLut { .. }
        ));
        assert_eq!(export.lut_values().unwrap(), &[-1.0, -1.0, 1.0]);
    }

    #[test]
    fn qat_norm_tile_rms_then_affine_clip_uses_per_tile_rms() {
        let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 0.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        let output = norm.forward(&[1.0, 1.0, 0.0, 2.0]).unwrap();

        assert_eq!(output, vec![1.0, 1.0, 0.0, 2.0_f32.sqrt()]);
        assert!(matches!(
            norm.export_norm_params().plan(),
            NormApproxPlan::TileRmsThenAffineClip { .. }
        ));
        assert_eq!(norm.export_norm_params().lut_values(), None);
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
}
