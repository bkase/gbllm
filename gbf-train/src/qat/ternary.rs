//! Burn-backed ternary linear QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{Q8_8Scale, TernaryLinearExport, TernaryLinearQat, TernaryLinearQatError};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam, burn_linear,
    float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape, ste_clamp,
    ste_replace_forward, ste_round,
};

#[derive(BurnModule, Debug)]
pub struct TernaryLinearBurnQat<B: BurnBackend> {
    full_precision_weights: BurnParam<BurnFloatTensor<B, 2>>,
    scale_factors: BurnParam<BurnFloatTensor<B, 1>>,
    bias: Option<BurnParam<BurnFloatTensor<B, 1>>>,
    #[module(skip)]
    core: TernaryLinearQat,
}

impl<B: BurnBackend> TernaryLinearBurnQat<B> {
    pub fn from_core(
        core: TernaryLinearQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, TernaryLinearBurnQatError> {
        let shape = core.shape();
        let full_precision_weights = float_tensor_from_vec(
            core.full_precision_weights().to_vec(),
            [shape.output_rows(), shape.input_cols()],
            device,
        )?;
        let scale_factors = float_tensor_from_vec(
            core.scales()
                .iter()
                .map(|scale| scale.to_f32())
                .collect::<Vec<_>>(),
            [shape.output_rows()],
            device,
        )?;
        let bias = core
            .bias()
            .map(|bias| float_tensor_from_vec(bias.to_vec(), [shape.output_rows()], device))
            .transpose()?;

        Ok(Self {
            full_precision_weights: BurnParam::from_tensor(full_precision_weights),
            scale_factors: BurnParam::from_tensor(scale_factors),
            bias: bias.map(BurnParam::from_tensor),
            core,
        })
    }

    pub fn core(&self) -> &TernaryLinearQat {
        &self.core
    }

    pub fn full_precision_weights(&self) -> BurnFloatTensor<B, 2> {
        self.full_precision_weights.val()
    }

    pub fn scale_factors(&self) -> BurnFloatTensor<B, 1> {
        self.scale_factors.val()
    }

    pub fn bias(&self) -> Option<BurnFloatTensor<B, 1>> {
        self.bias.as_ref().map(BurnParam::val)
    }

    pub fn fake_quant_forward(
        &self,
        input: BurnFloatTensor<B, 1>,
    ) -> Result<BurnFloatTensor<B, 1>, TernaryLinearBurnQatError> {
        let shape = self.core.shape();
        let input_shape = float_tensor_shape(&input);
        if input_shape != [shape.input_cols()] {
            return Err(TernaryLinearBurnQatError::InputShapeMismatch {
                expected: [shape.input_cols()],
                actual: input_shape,
            });
        }

        let weights = self.full_precision_weights();
        let device = weights.device();
        let thresholds = threshold_tensor(&self.core, &device)?;
        let hard_symbols = hard_ternary_symbols(weights.clone(), thresholds);
        let symbols = ste_replace_forward(weights, hard_symbols);
        let scales = fake_quant_q8_8_scales(self.scale_factors())
            .reshape([shape.output_rows(), 1])
            .repeat_dim(1, shape.input_cols());

        let projected_weights = symbols * scales;
        Ok(burn_linear(
            input,
            projected_weights.transpose(),
            self.bias(),
        ))
    }

    pub fn export_canonical(&self) -> Result<TernaryLinearExport, TernaryLinearBurnQatError> {
        let weights = float_tensor_into_vec(self.full_precision_weights().detach())?;
        let scales = float_tensor_into_vec(self.scale_factors().detach())?;
        let bias = self
            .bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;

        self.core
            .export_canonical_from_trained_state(&weights, &scales, bias.as_deref())
            .map_err(TernaryLinearBurnQatError::Model)
    }
}

#[derive(Debug)]
pub enum TernaryLinearBurnQatError {
    Adapter(BurnAdapterError),
    Model(TernaryLinearQatError),
    InputShapeMismatch {
        expected: [usize; 1],
        actual: [usize; 1],
    },
}

impl fmt::Display for TernaryLinearBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::InputShapeMismatch { expected, actual } => {
                write!(
                    f,
                    "input shape mismatch: expected {expected:?}, got {actual:?}"
                )
            }
        }
    }
}

impl Error for TernaryLinearBurnQatError {}

impl From<BurnAdapterError> for TernaryLinearBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<TernaryLinearQatError> for TernaryLinearBurnQatError {
    fn from(error: TernaryLinearQatError) -> Self {
        Self::Model(error)
    }
}

fn threshold_tensor<B: BurnBackend>(
    core: &TernaryLinearQat,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 2>, BurnAdapterError> {
    let shape = core.shape();
    let mut values = Vec::with_capacity(shape.weight_len());
    for threshold in core.thresholds() {
        values.extend(std::iter::repeat_n(threshold.value(), shape.input_cols()));
    }

    float_tensor_from_vec(values, [shape.output_rows(), shape.input_cols()], device)
}

fn hard_ternary_symbols<B: BurnBackend>(
    weights: BurnFloatTensor<B, 2>,
    thresholds: BurnFloatTensor<B, 2>,
) -> BurnFloatTensor<B, 2> {
    let positive = weights.clone().greater(thresholds.clone());
    let negative = weights.clone().lower(-thresholds);

    weights
        .zeros_like()
        .mask_fill(positive, 1.0f32)
        .mask_fill(negative, -1.0f32)
}

fn fake_quant_q8_8_scales<B: BurnBackend>(scales: BurnFloatTensor<B, 1>) -> BurnFloatTensor<B, 1> {
    let clamped = ste_clamp(scales, Q8_8Scale::ZERO.to_f32(), Q8_8Scale::MAX.to_f32());
    ste_round(clamped * Q8_8Scale::QUANTIZATION_SCALE) / Q8_8Scale::QUANTIZATION_SCALE
}

#[cfg(test)]
mod tests {
    use gbf_model::qat::{MatrixShape, TernaryThreshold, TernaryValue};

    use super::*;
    use crate::adapter::burn::{BurnNdArrayAutodiffBackend, BurnNdArrayBackend};

    #[test]
    fn burn_ternary_forward_export_and_gradients_share_owned_tensors() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = TernaryLinearQat::new(
            MatrixShape::new(1, 3).unwrap(),
            vec![-2.0, -0.1, 0.6],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(0.25).unwrap()],
        )
        .unwrap();
        let layer = TernaryLinearBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, 4.0], [3], &device).unwrap();

        let output = layer.fake_quant_forward(input).unwrap();
        let output_values = float_tensor_into_vec(output.clone().inner()).unwrap();
        let gradients = output.sum().backward();

        let weight_grad = layer
            .full_precision_weights()
            .grad(&gradients)
            .expect("weight gradient should exist");
        let scale_grad = layer
            .scale_factors()
            .grad(&gradients)
            .expect("scale gradient should exist");
        let export = layer.export_canonical().unwrap();

        assert_eq!(output_values, vec![0.75]);
        assert_eq!(
            float_tensor_into_vec(weight_grad).unwrap(),
            vec![0.25, 0.5, 1.0]
        );
        assert_eq!(float_tensor_into_vec(scale_grad).unwrap(), vec![3.0]);
        assert_eq!(
            export.ternary_values(),
            &[
                TernaryValue::Negative,
                TernaryValue::Zero,
                TernaryValue::Positive,
            ]
        );
        assert_eq!(export.projected_weights(), vec![-0.25, 0.0, 0.25]);
    }

    #[test]
    fn burn_ternary_rejects_wrong_input_shape_before_burn_matmul() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = TernaryLinearQat::new(
            MatrixShape::new(1, 3).unwrap(),
            vec![-2.0, -0.1, 0.6],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(0.25).unwrap()],
        )
        .unwrap();
        let layer = TernaryLinearBurnQat::<B>::from_core(core, &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0], [2], &device).unwrap();

        assert!(matches!(
            layer.fake_quant_forward(input),
            Err(TernaryLinearBurnQatError::InputShapeMismatch {
                expected: [3],
                actual: [2],
            })
        ));
    }
}
