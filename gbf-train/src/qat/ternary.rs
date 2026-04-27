//! Burn-backed ternary linear QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_artifact::weight_plan::TernaryWeightPlan;
use gbf_model::qat::{
    DEFAULT_SOFT_TERNARY_TEMPERATURE, MatrixShape, Q8_8Scale, QatHardnessControl, QuantHardness,
    TernaryLinearExport, TernaryLinearQat, TernaryLinearQatError, TernaryThreshold,
};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam, burn_linear,
    burn_sigmoid, float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape, ste_clamp,
    ste_replace_forward, ste_round,
};

#[derive(BurnModule, Debug)]
pub struct TernaryLinearBurnQat<B: BurnBackend> {
    full_precision_weights: BurnParam<BurnFloatTensor<B, 2>>,
    thresholds: BurnParam<BurnFloatTensor<B, 1>>,
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
        let thresholds = float_tensor_from_vec(
            core.thresholds()
                .iter()
                .map(|threshold| threshold.value())
                .collect::<Vec<_>>(),
            [shape.output_rows()],
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
            thresholds: BurnParam::from_tensor(thresholds),
            scale_factors: BurnParam::from_tensor(scale_factors),
            bias: bias.map(BurnParam::from_tensor),
            core,
        })
    }

    pub fn plan(&self) -> TernaryWeightPlan {
        self.core.plan()
    }

    pub fn shape(&self) -> MatrixShape {
        self.core.shape()
    }

    pub fn full_precision_weights(&self) -> BurnFloatTensor<B, 2> {
        self.full_precision_weights.val()
    }

    pub fn thresholds(&self) -> BurnFloatTensor<B, 1> {
        self.thresholds.val()
    }

    pub fn scale_factors(&self) -> BurnFloatTensor<B, 1> {
        self.scale_factors.val()
    }

    pub fn bias(&self) -> Option<BurnFloatTensor<B, 1>> {
        self.bias.as_ref().map(BurnParam::val)
    }

    pub fn fake_quant_forward<const D: usize>(
        &self,
        input: BurnFloatTensor<B, D>,
    ) -> Result<BurnFloatTensor<B, D>, TernaryLinearBurnQatError> {
        let shape = self.core.shape();
        let input_shape = float_tensor_shape(&input);
        let actual_last_dim = *input_shape
            .last()
            .expect("Burn tensors always carry a rank in their type");
        if actual_last_dim != shape.input_cols() {
            return Err(TernaryLinearBurnQatError::InputLastDimMismatch {
                expected: shape.input_cols(),
                actual: actual_last_dim,
                shape: input_shape.to_vec(),
            });
        }
        validate_finite_input(&input)?;

        if self.core.hardness() == QuantHardness::Off {
            return Ok(burn_linear(
                input,
                self.full_precision_weights().transpose(),
                self.bias(),
            ));
        }

        let weights = self.full_precision_weights();
        let thresholds =
            threshold_tensor(&self.core, fake_quant_q8_8_nonnegative(self.thresholds()));
        let symbols = match self.core.hardness() {
            QuantHardness::Off => unreachable!("off hardness returns before quantized path"),
            QuantHardness::Soft => soft_ternary_symbols(weights, thresholds),
            QuantHardness::Hard => {
                let hard_symbols = hard_ternary_symbols(weights.clone(), thresholds.clone());
                let weight_sign = weights.clone().sign().detach();
                let surrogate_symbols = weights - thresholds * weight_sign;
                ste_replace_forward(surrogate_symbols, hard_symbols)
            }
        };
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
        let thresholds = float_tensor_into_vec(self.thresholds().detach())?;
        let scales = float_tensor_into_vec(self.scale_factors().detach())?;
        let bias = self
            .bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;

        self.core
            .export_canonical_from_trained_state(&weights, &thresholds, &scales, bias.as_deref())
            .map_err(TernaryLinearBurnQatError::Model)
    }

    pub fn to_core_from_trained_state(
        &self,
    ) -> Result<TernaryLinearQat, TernaryLinearBurnQatError> {
        let weights = float_tensor_into_vec(self.full_precision_weights().detach())?;
        let thresholds = float_tensor_into_vec(self.thresholds().detach())?
            .into_iter()
            .map(TernaryThreshold::from_f32_clamped_q8_8)
            .collect::<Result<Vec<_>, _>>()?;
        let scales = float_tensor_into_vec(self.scale_factors().detach())?
            .into_iter()
            .map(Q8_8Scale::from_f32_clamped)
            .collect::<Result<Vec<_>, _>>()?;
        let bias = self
            .bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;

        TernaryLinearQat::new(self.shape(), weights, bias, thresholds, scales)
            .map(|mut core| {
                core.set_hardness(self.hardness());
                core
            })
            .map_err(TernaryLinearBurnQatError::Model)
    }
}

impl<B: BurnBackend> QatHardnessControl for TernaryLinearBurnQat<B> {
    fn hardness(&self) -> QuantHardness {
        self.core.hardness()
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        self.core.set_hardness(hardness);
    }
}

#[derive(Debug)]
pub enum TernaryLinearBurnQatError {
    Adapter(BurnAdapterError),
    Model(TernaryLinearQatError),
    InputLastDimMismatch {
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
}

impl fmt::Display for TernaryLinearBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::InputLastDimMismatch {
                expected,
                actual,
                shape,
            } => {
                write!(
                    f,
                    "input last dimension mismatch: expected {expected}, got {actual} in shape {shape:?}"
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
    thresholds: BurnFloatTensor<B, 1>,
) -> BurnFloatTensor<B, 2> {
    let shape = core.shape();
    thresholds
        .reshape([shape.output_rows(), 1])
        .repeat_dim(1, shape.input_cols())
}

fn validate_finite_input<B: BurnBackend, const D: usize>(
    input: &BurnFloatTensor<B, D>,
) -> Result<(), TernaryLinearBurnQatError> {
    let values = float_tensor_into_vec(input.clone().detach())?;
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(TernaryLinearQatError::NonFiniteInput { index }.into());
    }

    Ok(())
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

fn soft_ternary_symbols<B: BurnBackend>(
    weights: BurnFloatTensor<B, 2>,
    thresholds: BurnFloatTensor<B, 2>,
) -> BurnFloatTensor<B, 2> {
    let positive =
        burn_sigmoid((weights.clone() - thresholds.clone()) * DEFAULT_SOFT_TERNARY_TEMPERATURE);
    let negative = burn_sigmoid((-weights - thresholds) * DEFAULT_SOFT_TERNARY_TEMPERATURE);

    positive - negative
}

fn fake_quant_q8_8_scales<B: BurnBackend>(scales: BurnFloatTensor<B, 1>) -> BurnFloatTensor<B, 1> {
    let clamped = ste_clamp(scales, Q8_8Scale::ZERO.to_f32(), Q8_8Scale::MAX.to_f32());
    ste_round(clamped * Q8_8Scale::QUANTIZATION_SCALE) / Q8_8Scale::QUANTIZATION_SCALE
}

fn fake_quant_q8_8_nonnegative<B: BurnBackend>(
    values: BurnFloatTensor<B, 1>,
) -> BurnFloatTensor<B, 1> {
    let clamped = ste_clamp(values, Q8_8Scale::ZERO.to_f32(), Q8_8Scale::MAX.to_f32());
    ste_round(clamped * Q8_8Scale::QUANTIZATION_SCALE) / Q8_8Scale::QUANTIZATION_SCALE
}

#[cfg(test)]
mod tests {
    use gbf_artifact::sequence::{SequenceExportFacts, SequenceSemanticsSpec};
    use gbf_model::qat::{
        ExportVisitor, MatrixShape, QatModuleRef, TernaryThreshold, TernaryValue,
    };

    use super::*;
    use crate::adapter::burn::{
        BurnGradientsParams, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnOptimizer,
        adam_config,
    };

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
        let threshold_grad = layer
            .thresholds()
            .grad(&gradients)
            .expect("threshold gradient should exist");
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
        assert_eq!(float_tensor_into_vec(threshold_grad).unwrap(), vec![-0.25]);
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
    fn burn_ternary_one_step_train_export_round_trip_survives_burn_api() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = TernaryLinearQat::new(
            MatrixShape::new(1, 3).unwrap(),
            vec![-0.2, 0.1, 0.2],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(0.25).unwrap()],
        )
        .unwrap();
        let layer = TernaryLinearBurnQat::<B>::from_core(core, &device).unwrap();
        let initial_weights =
            float_tensor_into_vec(layer.full_precision_weights().detach()).unwrap();

        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 1.0, 1.0], [3], &device).unwrap();
        let target = float_tensor_from_vec::<B, 1>(vec![1.0], [1], &device).unwrap();
        let error = layer.fake_quant_forward(input).unwrap() - target;
        let loss = (error.clone() * error).sum();
        let gradients = loss.backward();
        let gradients = BurnGradientsParams::from_grads(gradients, &layer);

        let mut optimizer = adam_config().init::<B, TernaryLinearBurnQat<B>>();
        let trained = optimizer.step(1.0, layer, gradients);
        let trained_weights =
            float_tensor_into_vec(trained.full_precision_weights().detach()).unwrap();
        assert_ne!(
            trained_weights, initial_weights,
            "one optimizer step should update the Burn-owned training tensors"
        );
        assert!(trained_weights.iter().all(|value| value.is_finite()));

        let burn_export = trained.export_canonical().unwrap();
        let trained_core = trained.to_core_from_trained_state().unwrap();
        assert_eq!(
            burn_export.projected_weights(),
            trained_core.export_canonical().projected_weights()
        );

        let mut visitor = ExportVisitor::new(SequenceExportFacts::for_spec(
            SequenceSemanticsSpec::bounded_kv(16, 8).unwrap(),
        ));
        visitor
            .visit_module("projection", QatModuleRef::TernaryLinear(&trained_core))
            .unwrap();
        let artifact = visitor.finish().unwrap();
        let encoded = serde_json::to_vec(&artifact).unwrap();
        let decoded: gbf_model::qat::ExportedQatArtifact =
            serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded, artifact);
        assert_eq!(decoded.artifact_core_hash(), artifact.artifact_core_hash());
        assert_eq!(decoded.core.tensors().len(), 2);
        assert_eq!(decoded.visited_modules.len(), 1);
    }

    #[test]
    fn burn_ternary_batched_forward_uses_burn_linear_shape_rules() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = TernaryLinearQat::new(
            MatrixShape::new(2, 3).unwrap(),
            vec![-2.0, -0.1, 0.6, 0.25, 0.75, -0.8],
            Some(vec![0.5, -0.25]),
            vec![
                TernaryThreshold::new(0.5).unwrap(),
                TernaryThreshold::new(0.5).unwrap(),
            ],
            vec![
                Q8_8Scale::from_f32(0.25).unwrap(),
                Q8_8Scale::from_f32(0.5).unwrap(),
            ],
        )
        .unwrap();
        let layer = TernaryLinearBurnQat::<B>::from_core(core, &device).unwrap();
        let input =
            float_tensor_from_vec::<B, 2>(vec![1.0, 2.0, 4.0, 0.0, 1.0, -1.0], [2, 3], &device)
                .unwrap();

        let output = layer.fake_quant_forward(input).unwrap();

        assert_eq!(float_tensor_shape(&output), [2, 2]);
        assert_eq!(
            float_tensor_into_vec(output).unwrap(),
            vec![1.25, -1.25, 0.25, 0.75]
        );
    }

    #[test]
    fn burn_ternary_hardness_controls_off_soft_and_hard_forward() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let input = vec![1.0, 2.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();

        for hardness in [QuantHardness::Off, QuantHardness::Soft, QuantHardness::Hard] {
            let mut core = TernaryLinearQat::new(
                MatrixShape::new(2, 2).unwrap(),
                vec![
                    0.6, 0.0, //
                    0.0, -0.4,
                ],
                None,
                vec![TernaryThreshold::new(0.5).unwrap(); 2],
                vec![Q8_8Scale::from_f32(1.0).unwrap(); 2],
            )
            .unwrap();
            core.set_hardness(hardness);
            let layer = TernaryLinearBurnQat::<B>::from_core(core.clone(), &device).unwrap();

            let output = layer.fake_quant_forward(tensor.clone()).unwrap();

            assert_close(
                &float_tensor_into_vec(output).unwrap(),
                &core.inference_forward(&input).unwrap(),
                1.0e-6,
            );
        }
    }

    #[test]
    fn burn_ternary_hardness_can_change_after_adapter_construction() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let mut core = TernaryLinearQat::new(
            MatrixShape::new(1, 1).unwrap(),
            vec![0.6],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(1.0).unwrap()],
        )
        .unwrap();
        let mut layer = TernaryLinearBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let tensor = float_tensor_from_vec::<B, 1>(vec![1.0], [1], &device).unwrap();

        let hard = layer.fake_quant_forward(tensor.clone()).unwrap();
        layer.set_hardness(QuantHardness::Off);
        core.set_hardness(QuantHardness::Off);
        let expected_off = core.inference_forward(&[1.0]).unwrap();
        let off = layer.fake_quant_forward(tensor.clone()).unwrap();
        layer.set_hardness(QuantHardness::Soft);
        core.set_hardness(QuantHardness::Soft);
        let expected_soft = core.inference_forward(&[1.0]).unwrap();
        let soft = layer.fake_quant_forward(tensor.clone()).unwrap();

        assert_close(&float_tensor_into_vec(hard).unwrap(), &[1.0], f32::EPSILON);
        assert_close(&float_tensor_into_vec(off).unwrap(), &expected_off, 1.0e-6);
        assert_close(
            &float_tensor_into_vec(soft).unwrap(),
            &expected_soft,
            1.0e-6,
        );
    }

    #[test]
    fn burn_ternary_export_matches_core_projection_for_edge_cases() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let thresholds = [0.0, 0.5, 1.25];
        let scales = [0.25, 1.0, 3.5];

        for threshold in thresholds {
            let step = 1.0 / Q8_8Scale::QUANTIZATION_SCALE;
            let weights = vec![
                -threshold - step,
                -threshold,
                -0.0,
                0.0,
                threshold,
                threshold + step,
            ];

            for scale in scales {
                let core = TernaryLinearQat::new(
                    MatrixShape::new(1, weights.len()).unwrap(),
                    weights.clone(),
                    Some(vec![0.125]),
                    vec![TernaryThreshold::from_f32_clamped_q8_8(threshold).unwrap()],
                    vec![Q8_8Scale::from_f32(scale).unwrap()],
                )
                .unwrap();
                let core_export = core.export_canonical();
                let burn_export = TernaryLinearBurnQat::<B>::from_core(core, &device)
                    .unwrap()
                    .export_canonical()
                    .unwrap();

                assert_eq!(burn_export.plan(), core_export.plan());
                assert_eq!(burn_export.shape(), core_export.shape());
                assert_eq!(burn_export.ternary_values(), core_export.ternary_values());
                assert_eq!(burn_export.scales(), core_export.scales());
                assert_eq!(burn_export.bias_values(), core_export.bias_values());
                assert_eq!(
                    burn_export.projected_weights(),
                    core_export.projected_weights()
                );
            }
        }
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
            Err(TernaryLinearBurnQatError::InputLastDimMismatch {
                expected: 3,
                actual: 2,
                shape,
            })
            if shape == vec![2]
        ));

        let input = float_tensor_from_vec::<B, 1>(vec![1.0, f32::NAN, 2.0], [3], &device).unwrap();
        assert!(matches!(
            layer.fake_quant_forward(input),
            Err(TernaryLinearBurnQatError::Model(
                TernaryLinearQatError::NonFiniteInput { index: 1 }
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
