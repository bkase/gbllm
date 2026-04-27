//! Burn-backed top-1 router QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{
    RouterAuxLossWeights, RouterForwardOptions, RouterShape, RouterTrainMode, Top1RouterQat,
    Top1RouterQatError,
};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam, burn_linear,
    burn_softmax, float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape,
    ste_replace_forward,
};

#[derive(BurnModule, Debug)]
pub struct Top1RouterBurnQat<B: BurnBackend> {
    #[module(skip)]
    shape: RouterShape,
    input_projection: BurnParam<BurnFloatTensor<B, 2>>,
    input_bias: Option<BurnParam<BurnFloatTensor<B, 1>>>,
    expert_projection: BurnParam<BurnFloatTensor<B, 2>>,
    expert_bias: Option<BurnParam<BurnFloatTensor<B, 1>>>,
    #[module(skip)]
    aux_loss_weights: RouterAuxLossWeights,
}

impl<B: BurnBackend> Top1RouterBurnQat<B> {
    pub fn from_core(
        core: Top1RouterQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, Top1RouterBurnQatError> {
        let shape = core.shape();
        let input_projection = float_tensor_from_vec(
            core.input_projection().to_vec(),
            [shape.rank(), shape.d_model()],
            device,
        )?;
        let input_bias = core
            .input_bias()
            .map(|bias| float_tensor_from_vec(bias.to_vec(), [shape.rank()], device))
            .transpose()?;
        let expert_projection = float_tensor_from_vec(
            core.expert_projection().to_vec(),
            [shape.n_experts(), shape.rank()],
            device,
        )?;
        let expert_bias = core
            .expert_bias()
            .map(|bias| float_tensor_from_vec(bias.to_vec(), [shape.n_experts()], device))
            .transpose()?;

        Ok(Self {
            shape,
            input_projection: BurnParam::from_tensor(input_projection),
            input_bias: input_bias.map(BurnParam::from_tensor),
            expert_projection: BurnParam::from_tensor(expert_projection),
            expert_bias: expert_bias.map(BurnParam::from_tensor),
            aux_loss_weights: core.aux_loss_weights(),
        })
    }

    #[must_use]
    pub fn shape(&self) -> RouterShape {
        self.shape
    }

    #[must_use]
    pub fn input_projection(&self) -> BurnFloatTensor<B, 2> {
        self.input_projection.val()
    }

    #[must_use]
    pub fn input_bias(&self) -> Option<BurnFloatTensor<B, 1>> {
        self.input_bias.as_ref().map(BurnParam::val)
    }

    #[must_use]
    pub fn expert_projection(&self) -> BurnFloatTensor<B, 2> {
        self.expert_projection.val()
    }

    #[must_use]
    pub fn expert_bias(&self) -> Option<BurnFloatTensor<B, 1>> {
        self.expert_bias.as_ref().map(BurnParam::val)
    }

    #[must_use]
    pub fn aux_loss_weights(&self) -> RouterAuxLossWeights {
        self.aux_loss_weights
    }

    pub fn forward(
        &self,
        input: BurnFloatTensor<B, 1>,
        previous_distribution: Option<BurnFloatTensor<B, 1>>,
        options: &RouterForwardOptions,
        device: &BurnDevice<B>,
    ) -> Result<RouterBurnForwardOutput<B>, Top1RouterBurnQatError> {
        validate_router_input(self.shape, &input)?;
        validate_finite_router_input(&input)?;
        validate_previous_distribution(self.shape, previous_distribution.as_ref())?;
        validate_finite_previous_distribution(previous_distribution.as_ref())?;
        validate_router_options(self.shape, options)?;

        let hidden = burn_linear(
            input,
            self.input_projection().transpose(),
            self.input_bias(),
        );
        let logits = burn_linear(
            hidden,
            self.expert_projection().transpose(),
            self.expert_bias(),
        );
        let logits = add_logit_jitter(logits, options, device)?;
        let masked_logits = mask_dropped_experts(logits.clone(), options, device)?;
        let soft_probs = burn_softmax(masked_logits.clone(), 0);
        let expert_index = top1_index_from_logits(masked_logits.detach())?;
        let routing_weights = routing_weights_for_mode(
            soft_probs.clone(),
            expert_index,
            self.shape.n_experts(),
            options.mode(),
            device,
        )?;
        let aux_losses = self.compute_aux_losses(
            logits.clone(),
            soft_probs.clone(),
            expert_index,
            previous_distribution,
            device,
        )?;

        Ok(RouterBurnForwardOutput {
            expert_index,
            routing_weights,
            soft_probs,
            logits,
            aux_losses,
        })
    }

    pub fn to_core_from_trained_state(&self) -> Result<Top1RouterQat, Top1RouterBurnQatError> {
        let input_projection = float_tensor_into_vec(self.input_projection().detach())?;
        let input_bias = self
            .input_bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;
        let expert_projection = float_tensor_into_vec(self.expert_projection().detach())?;
        let expert_bias = self
            .expert_bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;

        Top1RouterQat::new_with_aux_loss_weights(
            self.shape,
            input_projection,
            input_bias,
            expert_projection,
            expert_bias,
            self.aux_loss_weights,
        )
        .map_err(Top1RouterBurnQatError::Model)
    }

    fn compute_aux_losses(
        &self,
        logits: BurnFloatTensor<B, 1>,
        soft_probs: BurnFloatTensor<B, 1>,
        expert_index: usize,
        previous_distribution: Option<BurnFloatTensor<B, 1>>,
        device: &BurnDevice<B>,
    ) -> Result<RouterBurnAuxLosses<B>, Top1RouterBurnQatError> {
        let max_logit = logits.clone().max().detach();
        let z = max_logit.clone() + (logits.clone() - max_logit).exp().sum().log();
        let z_loss = z.clone() * z;
        let expert_mask = one_hot_tensor(expert_index, self.shape.n_experts(), device)?;
        let token_balance_proxy_loss =
            (soft_probs.clone() * expert_mask).sum() * self.shape.n_experts() as f32;
        let temporal_smoothness_loss = if let Some(previous_distribution) = previous_distribution {
            let dot = (soft_probs * previous_distribution).sum();
            (dot.ones_like() - dot).clamp(0.0, 1.0)
        } else {
            z_loss.zeros_like()
        };

        Ok(RouterBurnAuxLosses {
            token_balance_proxy_loss,
            z_loss,
            temporal_smoothness_loss,
        })
    }
}

#[derive(Debug)]
pub struct RouterBurnForwardOutput<B: BurnBackend> {
    expert_index: usize,
    routing_weights: BurnFloatTensor<B, 1>,
    soft_probs: BurnFloatTensor<B, 1>,
    logits: BurnFloatTensor<B, 1>,
    aux_losses: RouterBurnAuxLosses<B>,
}

impl<B: BurnBackend> RouterBurnForwardOutput<B> {
    #[must_use]
    pub fn expert_index(&self) -> usize {
        self.expert_index
    }

    #[must_use]
    pub fn routing_weights(&self) -> BurnFloatTensor<B, 1> {
        self.routing_weights.clone()
    }

    #[must_use]
    pub fn soft_probs(&self) -> BurnFloatTensor<B, 1> {
        self.soft_probs.clone()
    }

    #[must_use]
    pub fn logits(&self) -> BurnFloatTensor<B, 1> {
        self.logits.clone()
    }

    #[must_use]
    pub fn aux_losses(&self) -> &RouterBurnAuxLosses<B> {
        &self.aux_losses
    }
}

#[derive(Debug)]
pub struct RouterBurnAuxLosses<B: BurnBackend> {
    token_balance_proxy_loss: BurnFloatTensor<B, 1>,
    z_loss: BurnFloatTensor<B, 1>,
    temporal_smoothness_loss: BurnFloatTensor<B, 1>,
}

impl<B: BurnBackend> RouterBurnAuxLosses<B> {
    #[must_use]
    pub fn token_balance_proxy_loss(&self) -> BurnFloatTensor<B, 1> {
        self.token_balance_proxy_loss.clone()
    }

    #[must_use]
    pub fn z_loss(&self) -> BurnFloatTensor<B, 1> {
        self.z_loss.clone()
    }

    #[must_use]
    pub fn temporal_smoothness_loss(&self) -> BurnFloatTensor<B, 1> {
        self.temporal_smoothness_loss.clone()
    }

    #[must_use]
    pub fn weighted_sum(&self, weights: RouterAuxLossWeights) -> BurnFloatTensor<B, 1> {
        self.token_balance_proxy_loss() * weights.token_balance_proxy()
            + self.z_loss() * weights.z_loss()
            + self.temporal_smoothness_loss() * weights.temporal_smoothness()
    }
}

#[derive(Debug)]
pub enum Top1RouterBurnQatError {
    Adapter(BurnAdapterError),
    Model(Top1RouterQatError),
    InputLastDimMismatch {
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
    PreviousDistributionLenMismatch {
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
}

impl fmt::Display for Top1RouterBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::InputLastDimMismatch {
                expected,
                actual,
                shape,
            } => write!(
                f,
                "router input last dimension mismatch: expected {expected}, got {actual} in shape {shape:?}"
            ),
            Self::PreviousDistributionLenMismatch {
                expected,
                actual,
                shape,
            } => write!(
                f,
                "previous router distribution length mismatch: expected {expected}, got {actual} in shape {shape:?}"
            ),
        }
    }
}

impl Error for Top1RouterBurnQatError {}

impl From<BurnAdapterError> for Top1RouterBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<Top1RouterQatError> for Top1RouterBurnQatError {
    fn from(error: Top1RouterQatError) -> Self {
        Self::Model(error)
    }
}

fn validate_router_input<B: BurnBackend>(
    shape: RouterShape,
    input: &BurnFloatTensor<B, 1>,
) -> Result<(), Top1RouterBurnQatError> {
    let tensor_shape = float_tensor_shape(input);
    if tensor_shape[0] != shape.d_model() {
        return Err(Top1RouterBurnQatError::InputLastDimMismatch {
            expected: shape.d_model(),
            actual: tensor_shape[0],
            shape: tensor_shape.to_vec(),
        });
    }

    Ok(())
}

fn validate_previous_distribution<B: BurnBackend>(
    shape: RouterShape,
    previous_distribution: Option<&BurnFloatTensor<B, 1>>,
) -> Result<(), Top1RouterBurnQatError> {
    let Some(previous_distribution) = previous_distribution else {
        return Ok(());
    };

    let tensor_shape = float_tensor_shape(previous_distribution);
    if tensor_shape[0] != shape.n_experts() {
        return Err(Top1RouterBurnQatError::PreviousDistributionLenMismatch {
            expected: shape.n_experts(),
            actual: tensor_shape[0],
            shape: tensor_shape.to_vec(),
        });
    }

    Ok(())
}

fn validate_finite_router_input<B: BurnBackend>(
    input: &BurnFloatTensor<B, 1>,
) -> Result<(), Top1RouterBurnQatError> {
    let values = float_tensor_into_vec(input.clone().detach())?;
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFiniteInput { index }.into());
    }

    Ok(())
}

fn validate_finite_previous_distribution<B: BurnBackend>(
    previous_distribution: Option<&BurnFloatTensor<B, 1>>,
) -> Result<(), Top1RouterBurnQatError> {
    let Some(previous_distribution) = previous_distribution else {
        return Ok(());
    };

    let values = float_tensor_into_vec(previous_distribution.clone().detach())?;
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFinitePreviousDistribution { index }.into());
    }

    Ok(())
}

fn validate_router_options(
    shape: RouterShape,
    options: &RouterForwardOptions,
) -> Result<(), Top1RouterBurnQatError> {
    if options.dropped_experts().len() != shape.n_experts() {
        return Err(Top1RouterQatError::DroppedExpertLenMismatch {
            expected: shape.n_experts(),
            actual: options.dropped_experts().len(),
        }
        .into());
    }
    if options.dropped_experts().iter().all(|&dropped| dropped) {
        return Err(Top1RouterQatError::AllExpertsDropped.into());
    }
    if options.logit_jitter().len() != shape.n_experts() {
        return Err(Top1RouterQatError::LogitJitterLenMismatch {
            expected: shape.n_experts(),
            actual: options.logit_jitter().len(),
        }
        .into());
    }
    if let Some(index) = options
        .logit_jitter()
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(Top1RouterQatError::NonFiniteLogitJitter { index }.into());
    }

    Ok(())
}

fn add_logit_jitter<B: BurnBackend>(
    logits: BurnFloatTensor<B, 1>,
    options: &RouterForwardOptions,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BurnAdapterError> {
    let jitter = float_tensor_from_vec(
        options.logit_jitter().to_vec(),
        [options.logit_jitter().len()],
        device,
    )?;

    Ok(logits + jitter)
}

fn mask_dropped_experts<B: BurnBackend>(
    logits: BurnFloatTensor<B, 1>,
    options: &RouterForwardOptions,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BurnAdapterError> {
    let mask = options
        .dropped_experts()
        .iter()
        .map(|&dropped| if dropped { f32::NEG_INFINITY } else { 0.0 })
        .collect::<Vec<_>>();
    let mask = float_tensor_from_vec(mask, [options.dropped_experts().len()], device)?;

    Ok(logits + mask)
}

fn routing_weights_for_mode<B: BurnBackend>(
    soft_probs: BurnFloatTensor<B, 1>,
    expert_index: usize,
    n_experts: usize,
    mode: RouterTrainMode,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BurnAdapterError> {
    match mode {
        RouterTrainMode::SoftTop1 => Ok(soft_probs),
        RouterTrainMode::HardTop1 => {
            let hard = one_hot_tensor(expert_index, n_experts, device)?;
            Ok(ste_replace_forward(soft_probs, hard))
        }
    }
}

fn one_hot_tensor<B: BurnBackend>(
    index: usize,
    len: usize,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BurnAdapterError> {
    let mut values = vec![0.0; len];
    values[index] = 1.0;

    float_tensor_from_vec(values, [len], device)
}

fn top1_index_from_logits<B: BurnBackend>(
    logits: BurnFloatTensor<B, 1>,
) -> Result<usize, BurnAdapterError> {
    let logits = float_tensor_into_vec(logits)?;
    Ok(logits
        .into_iter()
        .enumerate()
        .fold(
            (0, f32::NEG_INFINITY),
            |(best_index, best_value), (index, value)| {
                if value > best_value {
                    (index, value)
                } else {
                    (best_index, best_value)
                }
            },
        )
        .0)
}

#[cfg(test)]
mod tests {
    use gbf_model::qat::RouterShape;

    use super::*;
    use crate::adapter::burn::{
        BurnModuleMapper, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnParam,
        float_tensor_from_vec, float_tensor_into_vec,
    };

    #[test]
    fn burn_router_forward_matches_scalar_router_outputs() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = fixture_router();
        let layer = Top1RouterBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let options = RouterForwardOptions::hard_top1(4);
        let input = vec![1.0, 2.0, -1.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [3], &device).unwrap();

        let burn_output = layer.forward(tensor, None, &options, &device).unwrap();
        let scalar_output = core
            .forward_stateless(&input, None, &options)
            .expect("scalar router should accept fixture input");

        assert_eq!(burn_output.expert_index(), scalar_output.expert_index());
        assert_eq!(
            float_tensor_into_vec(burn_output.routing_weights()).unwrap(),
            scalar_output.routing_weights()
        );
        assert_close(
            &float_tensor_into_vec(burn_output.soft_probs()).unwrap(),
            scalar_output.soft_probs(),
            1.0e-6,
        );
        assert_close(
            &float_tensor_into_vec(burn_output.logits()).unwrap(),
            scalar_output.logits(),
            1.0e-6,
        );
    }

    #[test]
    fn burn_router_soft_mode_is_differentiable_through_projection_params() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let layer = Top1RouterBurnQat::<B>::from_core(fixture_router(), &device).unwrap();
        let options = RouterForwardOptions::soft_top1(4);
        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, -1.0], [3], &device).unwrap();

        let output = layer.forward(input, None, &options, &device).unwrap();
        let routing_reward =
            float_tensor_from_vec::<B, 1>(vec![0.0, 1.0, -0.5, 0.25], [4], &device).unwrap();
        let loss = (output.routing_weights() * routing_reward).sum();
        let gradients = loss.backward();
        let input_grad = layer
            .input_projection()
            .grad(&gradients)
            .expect("input projection gradient should exist");
        let expert_grad = layer
            .expert_projection()
            .grad(&gradients)
            .expect("expert projection gradient should exist");

        assert!(
            float_tensor_into_vec(input_grad)
                .unwrap()
                .iter()
                .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(expert_grad)
                .unwrap()
                .iter()
                .any(|value| value.abs() > 0.0)
        );
    }

    #[test]
    fn burn_router_rejects_shape_and_option_mismatches_before_forward() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = Top1RouterBurnQat::<B>::from_core(fixture_router(), &device).unwrap();
        let bad_input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0], [2], &device).unwrap();
        let options = RouterForwardOptions::hard_top1(4);

        assert!(matches!(
            layer.forward(bad_input, None, &options, &device),
            Err(Top1RouterBurnQatError::InputLastDimMismatch {
                expected: 3,
                actual: 2,
                ..
            })
        ));

        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, -1.0], [3], &device).unwrap();
        let bad_options = RouterForwardOptions::hard_top1(4).with_dropped_experts(vec![false; 3]);
        assert!(matches!(
            layer.forward(input, None, &bad_options, &device),
            Err(Top1RouterBurnQatError::Model(
                Top1RouterQatError::DroppedExpertLenMismatch {
                    expected: 4,
                    actual: 3,
                }
            ))
        ));

        let input = float_tensor_from_vec::<B, 1>(vec![1.0, f32::NAN, -1.0], [3], &device).unwrap();
        assert!(matches!(
            layer.forward(input, None, &options, &device),
            Err(Top1RouterBurnQatError::Model(
                Top1RouterQatError::NonFiniteInput { index: 1 }
            ))
        ));

        let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, -1.0], [3], &device).unwrap();
        let previous =
            float_tensor_from_vec::<B, 1>(vec![0.25, f32::INFINITY, 0.25, 0.25], [4], &device)
                .unwrap();
        assert!(matches!(
            layer.forward(input, Some(previous), &options, &device),
            Err(Top1RouterBurnQatError::Model(
                Top1RouterQatError::NonFinitePreviousDistribution { index: 1 }
            ))
        ));
    }

    #[test]
    fn burn_router_export_handoff_uses_burn_owned_tensors() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let original = fixture_router();
        let layer = Top1RouterBurnQat::<B>::from_core(original.clone(), &device).unwrap();
        let mut mapper = AddToFloatParams(0.125);
        let layer = layer.map(&mut mapper);
        let exported = layer.to_core_from_trained_state().unwrap();

        assert_close(
            exported.input_projection(),
            &add_delta(original.input_projection(), 0.125),
            0.0,
        );
        assert_close(
            exported.input_bias().unwrap(),
            &add_delta(original.input_bias().unwrap(), 0.125),
            0.0,
        );
        assert_close(
            exported.expert_projection(),
            &add_delta(original.expert_projection(), 0.125),
            0.0,
        );
        assert_close(
            exported.expert_bias().unwrap(),
            &add_delta(original.expert_bias().unwrap(), 0.125),
            0.0,
        );
    }

    #[test]
    fn burn_router_z_loss_uses_stable_scalar_logsumexp_oracle() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = large_logit_router();
        let layer = Top1RouterBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let options = RouterForwardOptions::soft_top1(2);
        let input = [1.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.to_vec(), [1], &device).unwrap();

        let burn_output = layer.forward(tensor, None, &options, &device).unwrap();
        let burn_z_loss = float_tensor_into_vec(burn_output.aux_losses().z_loss()).unwrap()[0];
        let scalar_z_loss = core
            .forward_stateless(&input, None, &options)
            .unwrap()
            .aux_losses()
            .z_loss();

        assert!(burn_z_loss.is_finite());
        assert_close(&[burn_z_loss], &[scalar_z_loss], 1.0e-2);
    }

    fn fixture_router() -> Top1RouterQat {
        Top1RouterQat::new(
            RouterShape::new(3, 4, 2).unwrap(),
            vec![
                1.0, 0.0, -1.0, //
                0.0, 1.0, 0.5,
            ],
            Some(vec![0.0, 0.25]),
            vec![
                1.0, 0.0, //
                0.0, 1.0, //
                -1.0, 0.5, //
                0.25, -0.5,
            ],
            Some(vec![0.0, 0.1, -0.2, 0.0]),
        )
        .unwrap()
    }

    fn large_logit_router() -> Top1RouterQat {
        Top1RouterQat::new(
            RouterShape::new(1, 2, 1).unwrap(),
            vec![100.0],
            None,
            vec![1.0, 1.0],
            Some(vec![0.0, -1.0]),
        )
        .unwrap()
    }

    fn add_delta(values: &[f32], delta: f32) -> Vec<f32> {
        values.iter().map(|value| value + delta).collect()
    }

    struct AddToFloatParams(f32);

    impl<B: BurnBackend> BurnModuleMapper<B> for AddToFloatParams {
        fn map_float<const D: usize>(
            &mut self,
            param: BurnParam<BurnFloatTensor<B, D>>,
        ) -> BurnParam<BurnFloatTensor<B, D>> {
            param.map(|tensor| tensor + self.0)
        }
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
