//! Canonical deployable normalization approximation plans.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NormPlan {
    None,
    AffineClipLut(AffineClipLutPlan),
    TileRmsThenAffineClip(TileRmsThenAffineClipPlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AffineClipLutPlan {
    pub affine: NormAffineParams,
    pub clip: NormClipBounds,
    pub lut: NormLutSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TileRmsThenAffineClipPlan {
    pub tile: NormTileRmsSpec,
    pub affine: NormAffineParams,
    pub clip: NormClipBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormAffineParams {
    pub scale: f32,
    pub bias: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormClipBounds {
    pub lo: f32,
    pub hi: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormLutSpec {
    pub input_lo: f32,
    pub input_hi: f32,
    pub entries: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormTileRmsSpec {
    pub tile_width: u16,
    pub epsilon: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NormExportParams {
    None,
    AffineClipLut {
        plan: AffineClipLutPlan,
        lut_values: Vec<f32>,
    },
    TileRmsThenAffineClip {
        plan: TileRmsThenAffineClipPlan,
    },
}

impl NormPlan {
    #[must_use]
    pub const fn affine_clip_lut(
        affine: NormAffineParams,
        clip: NormClipBounds,
        lut: NormLutSpec,
    ) -> Self {
        Self::AffineClipLut(AffineClipLutPlan { affine, clip, lut })
    }

    #[must_use]
    pub const fn tile_rms_then_affine_clip(
        tile: NormTileRmsSpec,
        affine: NormAffineParams,
        clip: NormClipBounds,
    ) -> Self {
        Self::TileRmsThenAffineClip(TileRmsThenAffineClipPlan { tile, affine, clip })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_plan_round_trips_affine_clip_lut_export_params() {
        let export = NormExportParams::AffineClipLut {
            plan: AffineClipLutPlan {
                affine: NormAffineParams {
                    scale: 2.0,
                    bias: -1.0,
                },
                clip: NormClipBounds { lo: -1.0, hi: 1.0 },
                lut: NormLutSpec {
                    input_lo: -1.0,
                    input_hi: 1.0,
                    entries: 3,
                },
            },
            lut_values: vec![-1.0, -1.0, 1.0],
        };

        let encoded = serde_json::to_string(&export).expect("norm export serializes");
        let decoded: NormExportParams =
            serde_json::from_str(&encoded).expect("norm export deserializes");

        assert_eq!(decoded, export);
    }

    #[test]
    fn norm_plan_round_trips_tile_rms_export_params_without_lut_nullability() {
        let export = NormExportParams::TileRmsThenAffineClip {
            plan: TileRmsThenAffineClipPlan {
                tile: NormTileRmsSpec {
                    tile_width: 8,
                    epsilon: 1.0e-5,
                },
                affine: NormAffineParams {
                    scale: 1.0,
                    bias: 0.0,
                },
                clip: NormClipBounds { lo: -2.0, hi: 2.0 },
            },
        };

        let encoded = serde_json::to_string(&export).expect("norm export serializes");
        let decoded: NormExportParams =
            serde_json::from_str(&encoded).expect("norm export deserializes");

        assert_eq!(decoded, export);
        assert!(!encoded.contains("lut_values"));
    }
}
