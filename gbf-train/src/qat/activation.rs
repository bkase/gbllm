//! Burn-backed activation fake-quantization adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{
    ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
    ActivationRangeMode, ActivationRangeModeKind,
};

use crate::adapter::burn::{BurnBackend, BurnFloatTensor, ste_clamp, ste_round};

#[derive(Debug, Clone, PartialEq)]
pub struct ActFakeQuantBurnQat {
    core: ActFakeQuant,
}

impl ActFakeQuantBurnQat {
    pub fn from_core(core: ActFakeQuant) -> Result<Self, ActFakeQuantBurnQatError> {
        if !matches!(core.range_mode(), ActivationRangeMode::Fixed(_)) {
            return Err(ActFakeQuantBurnQatError::UnsupportedRangeMode {
                mode: core.range_mode().kind(),
            });
        }

        Ok(Self { core })
    }

    #[must_use]
    pub fn core(&self) -> &ActFakeQuant {
        &self.core
    }

    #[must_use]
    pub fn export_range(&self) -> ActivationRange {
        self.core.export_range()
    }

    #[must_use]
    pub fn quant_format(&self) -> ActivationQuantFormat {
        self.core.quant_format()
    }

    pub fn fake_quant_forward<B: BurnBackend, const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
        mode: ActivationForwardMode,
    ) -> BurnFloatTensor<B, D> {
        let spec = self.core.forward_spec(mode);
        if !spec.enabled() {
            return input;
        }

        let qmax = f32::from(spec.quant_steps());
        match spec.quant_format() {
            ActivationQuantFormat::Int8 => fake_quant_signed(input, spec.range(), qmax),
            ActivationQuantFormat::UInt8 | ActivationQuantFormat::UInt4 => {
                fake_quant_unsigned(input, spec.range(), qmax)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActFakeQuantBurnQatError {
    UnsupportedRangeMode { mode: ActivationRangeModeKind },
}

impl fmt::Display for ActFakeQuantBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRangeMode { mode } => {
                write!(
                    f,
                    "Burn activation fake-quant currently supports fixed ranges only, got {mode}"
                )
            }
        }
    }
}

impl Error for ActFakeQuantBurnQatError {}

fn fake_quant_signed<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    range: ActivationRange,
    qmax: f32,
) -> BurnFloatTensor<B, D> {
    let max_abs = range
        .lo()
        .abs()
        .max(range.hi().abs())
        .max(f32::MIN_POSITIVE);
    let clamped = ste_clamp(input, range.lo(), range.hi());
    let quantized = ste_clamp(ste_round((clamped / max_abs) * qmax), -qmax, qmax);
    ste_clamp((quantized / qmax) * max_abs, range.lo(), range.hi())
}

fn fake_quant_unsigned<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    range: ActivationRange,
    qmax: f32,
) -> BurnFloatTensor<B, D> {
    let width = (range.hi() - range.lo()).max(f32::MIN_POSITIVE);
    let clamped = ste_clamp(input, range.lo(), range.hi());
    let normalized = (clamped - range.lo()) / width;
    let quantized = ste_clamp(ste_round(normalized * qmax), 0.0, qmax);
    ste_clamp(
        (quantized / qmax) * width + range.lo(),
        range.lo(),
        range.hi(),
    )
}

#[cfg(test)]
mod tests {
    use gbf_model::qat::{ActivationRangeMode, EmaDecay};

    use super::*;
    use crate::adapter::burn::{
        BurnDevice, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, float_tensor_from_vec,
        float_tensor_into_vec,
    };

    #[test]
    fn burn_activation_forward_matches_core_scalar_projection() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 0.5).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core.clone()).unwrap();
        let input = vec![-2.0, -0.25, 0.0, 0.25, 2.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [5], &device).unwrap();

        let output = layer.fake_quant_forward(tensor, ActivationForwardMode::Train);

        assert_eq!(
            float_tensor_into_vec(output).unwrap(),
            core.inference_forward(&input, ActivationForwardMode::Train)
                .unwrap()
        );
    }

    #[test]
    fn burn_activation_eval_passthrough_returns_input_when_configured() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap()
        .with_eval_passthrough(true);
        let layer = ActFakeQuantBurnQat::from_core(core).unwrap();
        let input = vec![-1.0, 0.25, 2.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [3], &device).unwrap();

        let output = layer.fake_quant_forward(tensor, ActivationForwardMode::Eval);

        assert_eq!(float_tensor_into_vec(output).unwrap(), input);
    }

    #[test]
    fn burn_activation_uses_clipped_input_gradients() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![-2.0, -0.25, 0.25, 2.0], [4], &device)
            .unwrap()
            .require_grad();

        let output = layer.fake_quant_forward(input.clone(), ActivationForwardMode::Train);
        let gradients = output.sum().backward();
        let input_grad = input.grad(&gradients).expect("input gradient should exist");

        assert_eq!(
            float_tensor_into_vec(input_grad).unwrap(),
            vec![0.0, 1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn burn_activation_rejects_dynamic_range_modes_until_state_is_owned() {
        let learned = ActFakeQuant::new(
            ActivationRangeMode::Learned(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt4,
        )
        .unwrap();
        let ema = ActFakeQuant::new(
            ActivationRangeMode::Ema {
                range: ActivationRange::new(-1.0, 1.0).unwrap(),
                decay: EmaDecay::new(0.25).unwrap(),
            },
            ActivationQuantFormat::UInt8,
        )
        .unwrap();

        assert_eq!(
            ActFakeQuantBurnQat::from_core(learned),
            Err(ActFakeQuantBurnQatError::UnsupportedRangeMode {
                mode: ActivationRangeModeKind::Learned
            })
        );
        assert_eq!(
            ActFakeQuantBurnQat::from_core(ema),
            Err(ActFakeQuantBurnQatError::UnsupportedRangeMode {
                mode: ActivationRangeModeKind::Ema
            })
        );
    }

    #[test]
    fn burn_activation_uint4_forward_matches_core_scalar_projection() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-0.25, 0.75).unwrap()),
            ActivationQuantFormat::UInt4,
        )
        .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core.clone()).unwrap();
        let input = vec![-0.25, -0.15, 0.25, 0.45, 0.75];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [5], &device).unwrap();

        let output = layer.fake_quant_forward(tensor, ActivationForwardMode::Train);

        assert_close(
            &float_tensor_into_vec(output).unwrap(),
            &core
                .inference_forward(&input, ActivationForwardMode::Train)
                .unwrap(),
            f32::EPSILON,
        );
    }

    #[test]
    fn burn_activation_tiny_ranges_stay_finite() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0e-40, 1.0e-40).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core).unwrap();
        let tensor = float_tensor_from_vec::<B, 1>(vec![1.0e-40], [1], &device).unwrap();

        let output = layer.fake_quant_forward(tensor, ActivationForwardMode::Train);
        let values = float_tensor_into_vec(output).unwrap();

        assert!(values[0].is_finite());
        assert!((-1.0e-40..=1.0e-40).contains(&values[0]));
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
