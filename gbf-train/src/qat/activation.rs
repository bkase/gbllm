//! Burn-backed activation fake-quantization adapter.

use gbf_model::qat::{ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange};

use crate::adapter::burn::{BurnBackend, BurnFloatTensor, ste_clamp, ste_round};

#[derive(Debug, Clone, PartialEq)]
pub struct ActFakeQuantBurnQat {
    core: ActFakeQuant,
}

impl ActFakeQuantBurnQat {
    #[must_use]
    pub fn from_core(core: ActFakeQuant) -> Self {
        Self { core }
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

        match spec.quant_format() {
            ActivationQuantFormat::Int8 => fake_quant_signed(input, spec.range(), 127.0),
            ActivationQuantFormat::UInt8 => fake_quant_unsigned(input, spec.range(), 255.0),
            ActivationQuantFormat::Int4 => fake_quant_unsigned(input, spec.range(), 15.0),
        }
    }
}

fn fake_quant_signed<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    range: ActivationRange,
    qmax: f32,
) -> BurnFloatTensor<B, D> {
    let max_abs = range.lo().abs().max(range.hi().abs());
    let clamped = ste_clamp(input, range.lo(), range.hi());
    let quantized = ste_clamp(ste_round((clamped / max_abs) * qmax), -qmax, qmax);
    ste_clamp((quantized / qmax) * max_abs, range.lo(), range.hi())
}

fn fake_quant_unsigned<B: BurnBackend, const D: usize>(
    input: BurnFloatTensor<B, D>,
    range: ActivationRange,
    qmax: f32,
) -> BurnFloatTensor<B, D> {
    let width = range.hi() - range.lo();
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
        let layer = ActFakeQuantBurnQat::from_core(core.clone());
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
        let layer = ActFakeQuantBurnQat::from_core(core);
        let input = vec![-1.0, 0.25, 2.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [3], &device).unwrap();

        let output = layer.fake_quant_forward(tensor, ActivationForwardMode::Eval);

        assert_eq!(float_tensor_into_vec(output).unwrap(), input);
    }

    #[test]
    fn burn_activation_ste_preserves_input_gradients() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core);
        let input = float_tensor_from_vec::<B, 1>(vec![-2.0, -0.25, 0.25, 2.0], [4], &device)
            .unwrap()
            .require_grad();

        let output = layer.fake_quant_forward(input.clone(), ActivationForwardMode::Train);
        let gradients = output.sum().backward();
        let input_grad = input.grad(&gradients).expect("input gradient should exist");

        assert_eq!(
            float_tensor_into_vec(input_grad).unwrap(),
            vec![1.0, 1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn burn_activation_exports_current_ema_range_from_core() {
        let mut core = ActFakeQuant::new(
            ActivationRangeMode::Ema {
                range: ActivationRange::new(-1.0, 1.0).unwrap(),
                decay: EmaDecay::new(0.25).unwrap(),
            },
            ActivationQuantFormat::Int4,
        )
        .unwrap();
        core.update_ema_range(ActivationRange::new(-5.0, 3.0).unwrap())
            .unwrap();
        let layer = ActFakeQuantBurnQat::from_core(core);
        let range = layer.export_range();

        assert_eq!(range.lo(), -4.0);
        assert_eq!(range.hi(), 2.5);
        assert_eq!(layer.quant_format(), ActivationQuantFormat::Int4);
    }
}
