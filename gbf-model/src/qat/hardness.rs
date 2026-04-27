//! Per-phase QAT hardness controls shared by model and training config.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantHardness {
    /// Disable the quantized path for this phase component.
    Off,
    /// Enable train-time soft/fake quantization.
    Soft,
    /// Require the hard quantized path for this phase component.
    Hard,
}

pub const DEFAULT_SOFT_TERNARY_TEMPERATURE: f32 = 4.0;
pub const DEFAULT_SOFT_BLEND: f32 = 0.5;

impl fmt::Display for QuantHardness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Soft => f.write_str("soft"),
            Self::Hard => f.write_str("hard"),
        }
    }
}

pub trait QatHardnessControl {
    fn hardness(&self) -> QuantHardness;
    fn set_hardness(&mut self, hardness: QuantHardness);
}

#[cfg(test)]
mod tests {
    use crate::qat::{
        ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
        ActivationRangeMode, AffineParams, ExpertBlockQat, ExpertForwardOptions, ExpertQat,
        ExpertQatForwardMode, MatrixShape, NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale,
        TernaryLinearQat, TernaryThreshold, TileRmsSpec,
    };

    use super::*;

    #[test]
    fn qat_hardness_controls_ternary_linear_forward_and_is_idempotent() {
        let mut layer = ternary_layer();
        let input = [1.0, 2.0];

        assert_eq!(layer.hardness(), QuantHardness::Hard);
        assert_eq!(layer.inference_forward(&input).unwrap(), vec![1.0, -2.0]);

        layer.set_hardness(QuantHardness::Off);
        layer.set_hardness(QuantHardness::Off);
        assert_eq!(layer.hardness(), QuantHardness::Off);
        assert_eq!(layer.inference_forward(&input).unwrap(), vec![0.6, -1.2]);

        layer.set_hardness(QuantHardness::Soft);
        let soft = layer.inference_forward(&input).unwrap();
        assert!(soft[0] > 0.0 && soft[0] < 1.0);
        assert!(soft[1] < 0.0 && soft[1] > -2.0);

        layer.set_hardness(QuantHardness::Hard);
        assert_eq!(layer.inference_forward(&input).unwrap(), vec![1.0, -2.0]);
    }

    #[test]
    fn qat_hardness_controls_activation_and_norm_forward() {
        let mut activation = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt4,
        )
        .unwrap();

        activation.set_hardness(QuantHardness::Off);
        assert_eq!(
            activation
                .inference_forward(&[-1.0, 0.5, 2.0], ActivationForwardMode::Train)
                .unwrap(),
            vec![-1.0, 0.5, 2.0]
        );
        activation.set_hardness(QuantHardness::Soft);
        assert_eq!(
            activation
                .inference_forward(&[-1.0, 0.5, 2.0], ActivationForwardMode::Train)
                .unwrap(),
            vec![0.0, 0.516_666_65, 1.0]
        );
        activation.set_hardness(QuantHardness::Hard);
        assert_eq!(
            activation
                .inference_forward(&[-1.0, 0.5, 2.0], ActivationForwardMode::Train)
                .unwrap(),
            vec![0.0, 0.533_333_36, 1.0]
        );

        let mut norm = NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(2.0, 0.0).unwrap(),
            clip: NormClip::new(-1.0, 1.0).unwrap(),
            lut: crate::qat::LutSpec::new(-1.0, 1.0, 3).unwrap(),
        });
        norm.set_hardness(QuantHardness::Off);
        assert_eq!(norm.forward(&[0.25]).unwrap(), vec![0.5]);
        norm.set_hardness(QuantHardness::Soft);
        assert_eq!(norm.forward(&[0.25]).unwrap(), vec![0.25]);
        norm.set_hardness(QuantHardness::Hard);
        assert_eq!(norm.forward(&[0.25]).unwrap(), vec![0.0]);
    }

    #[test]
    fn qat_hardness_controls_tile_rms_norm_forward() {
        let mut norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(1, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-2.0, 2.0).unwrap(),
        });

        norm.set_hardness(QuantHardness::Off);
        let off = norm.forward(&[3.0, 4.0]).unwrap();
        norm.set_hardness(QuantHardness::Soft);
        let soft = norm.forward(&[3.0, 4.0]).unwrap();
        norm.set_hardness(QuantHardness::Hard);
        let hard = norm.forward(&[3.0, 4.0]).unwrap();

        assert!(off[0] < soft[0] && soft[0] < hard[0]);
        assert!(off[1] > soft[1] && soft[1] > hard[1]);
    }

    #[test]
    fn qat_hardness_maps_phase_values_to_expert_forward_options() {
        let off = ExpertForwardOptions::for_hardness(QuantHardness::Off, QuantHardness::Off);
        assert_eq!(off.expert_qat(), ExpertQatForwardMode::FullPrecision);
        assert_eq!(off.activation(), ActivationForwardMode::Passthrough);

        let soft = ExpertForwardOptions::for_hardness(QuantHardness::Soft, QuantHardness::Soft);
        assert_eq!(soft.expert_qat(), ExpertQatForwardMode::HardQuantized);
        assert_eq!(soft.activation(), ActivationForwardMode::Train);

        let hard = ExpertForwardOptions::for_hardness(QuantHardness::Hard, QuantHardness::Hard);
        assert_eq!(hard.expert_qat(), ExpertQatForwardMode::HardQuantized);
        assert_eq!(hard.activation(), ActivationForwardMode::Train);
    }

    #[test]
    fn qat_hardness_applies_to_expert_block_phase_transitions() {
        let mut block = ExpertBlockQat::without_shared_dense(vec![expert()]).unwrap();
        let input = [1.0];

        let hard = block.forward(&input, 0).unwrap();

        block.set_hardness(QuantHardness::Off, QuantHardness::Off);
        block.set_hardness(QuantHardness::Off, QuantHardness::Off);
        let off = block
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::for_hardness(QuantHardness::Off, QuantHardness::Off),
            )
            .unwrap();

        block.set_hardness(QuantHardness::Soft, QuantHardness::Soft);
        let soft = block
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::for_hardness(QuantHardness::Soft, QuantHardness::Soft),
            )
            .unwrap();

        assert_ne!(off, soft);
        assert!(soft[0] < hard[0]);
    }

    fn ternary_layer() -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(2, 2).unwrap(),
            vec![
                0.6, 0.0, //
                0.0, -0.6,
            ],
            None,
            vec![TernaryThreshold::new(0.5).unwrap(); 2],
            vec![Q8_8Scale::from_f32(1.0).unwrap(); 2],
        )
        .unwrap()
    }

    fn expert() -> ExpertQat {
        ExpertQat::new(
            TernaryLinearQat::new(
                MatrixShape::new(1, 1).unwrap(),
                vec![0.6],
                None,
                vec![TernaryThreshold::new(0.5).unwrap()],
                vec![Q8_8Scale::from_f32(1.0).unwrap()],
            )
            .unwrap(),
            ActFakeQuant::new(
                ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
                ActivationQuantFormat::UInt4,
            )
            .unwrap(),
            TernaryLinearQat::new(
                MatrixShape::new(1, 1).unwrap(),
                vec![0.6],
                None,
                vec![TernaryThreshold::new(0.5).unwrap()],
                vec![Q8_8Scale::from_f32(1.0).unwrap()],
            )
            .unwrap(),
        )
        .unwrap()
    }
}
