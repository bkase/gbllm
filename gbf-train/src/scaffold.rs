//! Sequence-state training scaffold dispatch.
//!
//! This module is intentionally generic over the training-side sequence block.
//! Callers pass a block implementing the fixed Burn tensor boundary; the
//! scaffold does not inspect `SequenceSemanticsSpec` variants.

use crate::adapter::burn::{BurnBackend, BurnDevice, BurnFloatTensor};
use crate::sequence::{
    BoundedKvBurnQat, BoundedKvBurnQatError, BoundedKvBurnRun, LinearStateBurnQat,
    LinearStateBurnQatError, LinearStateBurnRun,
};

#[derive(Debug)]
pub struct SequenceScaffoldRun<B: BurnBackend> {
    activations: BurnFloatTensor<B, 2>,
    final_state: BurnFloatTensor<B, 1>,
}

impl<B: BurnBackend> SequenceScaffoldRun<B> {
    #[must_use]
    pub fn activations(&self) -> BurnFloatTensor<B, 2> {
        self.activations.clone()
    }

    #[must_use]
    pub fn final_state(&self) -> BurnFloatTensor<B, 1> {
        self.final_state.clone()
    }
}

pub trait SequenceScaffoldBlock<B: BurnBackend> {
    type Error;

    fn zero_state(&self, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1>;

    fn train_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error>;

    fn eval_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error>;
}

pub fn train_sequence_block<B, S>(
    block: &S,
    input: BurnFloatTensor<B, 2>,
    device: &BurnDevice<B>,
) -> Result<SequenceScaffoldRun<B>, S::Error>
where
    B: BurnBackend,
    S: SequenceScaffoldBlock<B>,
{
    let state = block.zero_state(device);
    block.train_forward(input, state)
}

pub fn eval_sequence_block<B, S>(
    block: &S,
    input: BurnFloatTensor<B, 2>,
    device: &BurnDevice<B>,
) -> Result<SequenceScaffoldRun<B>, S::Error>
where
    B: BurnBackend,
    S: SequenceScaffoldBlock<B>,
{
    let state = block.zero_state(device);
    block.eval_forward(input, state)
}

impl<B: BurnBackend> SequenceScaffoldBlock<B> for LinearStateBurnQat<B> {
    type Error = LinearStateBurnQatError;

    fn zero_state(&self, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
        Self::zero_state(self, device)
    }

    fn train_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error> {
        let run = Self::train_forward(self, input, initial_state)?;
        Ok(run.into())
    }

    fn eval_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error> {
        let run = Self::eval_forward(self, input, initial_state)?;
        Ok(run.into())
    }
}

impl<B: BurnBackend> SequenceScaffoldBlock<B> for BoundedKvBurnQat<B> {
    type Error = BoundedKvBurnQatError;

    fn zero_state(&self, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
        Self::zero_state(self, device)
    }

    fn train_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error> {
        let run = Self::train_forward(self, input, initial_state)?;
        Ok(run.into())
    }

    fn eval_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<SequenceScaffoldRun<B>, Self::Error> {
        let run = Self::eval_forward(self, input, initial_state)?;
        Ok(run.into())
    }
}

impl<B: BurnBackend> From<LinearStateBurnRun<B>> for SequenceScaffoldRun<B> {
    fn from(run: LinearStateBurnRun<B>) -> Self {
        let (activations, final_state) = run.into_parts();
        Self {
            activations,
            final_state,
        }
    }
}

impl<B: BurnBackend> From<BoundedKvBurnRun<B>> for SequenceScaffoldRun<B> {
    fn from(run: BoundedKvBurnRun<B>) -> Self {
        let (activations, final_state) = run.into_parts();
        Self {
            activations,
            final_state,
        }
    }
}

#[cfg(test)]
pub mod dispatch {
    use gbf_model::qat::{
        ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
        MatrixShape, NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale, TernaryLinearQat,
        TernaryThreshold,
    };
    use gbf_model::sequence::{
        BoundedKvBlock, BoundedKvBlockConfig, LinearStateBlock, LinearStateBlockConfig,
    };

    use super::*;
    use crate::adapter::burn::{
        BurnDevice, BurnNdArrayBackend, float_tensor_from_vec, float_tensor_into_vec,
    };

    #[test]
    fn scaffold_dispatch_runs_linear_state_and_bounded_kv_without_variant_match() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let linear = LinearStateBurnQat::<B>::from_core(linear_state_block(), &device).unwrap();
        let bounded = BoundedKvBurnQat::<B>::from_core(bounded_kv_block(), &device).unwrap();
        let input = vec![
            1.0, 0.0, //
            0.0, 1.0,
        ];

        let linear_run = train_sequence_block(
            &linear,
            float_tensor_from_vec::<B, 2>(input.clone(), [2, 2], &device).unwrap(),
            &device,
        )
        .unwrap();
        let bounded_run = train_sequence_block(
            &bounded,
            float_tensor_from_vec::<B, 2>(input, [2, 2], &device).unwrap(),
            &device,
        )
        .unwrap();

        assert_eq!(
            float_tensor_into_vec(linear_run.activations())
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            float_tensor_into_vec(bounded_run.activations())
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            float_tensor_into_vec(linear_run.final_state())
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            float_tensor_into_vec(bounded_run.final_state())
                .unwrap()
                .len(),
            6
        );
    }

    fn linear_state_block() -> LinearStateBlock {
        let mut block = LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            identity_norm(),
            activation(),
            identity_ternary(),
            identity_ternary(),
            activation(),
        )
        .unwrap();
        block.set_hardness(
            gbf_model::qat::QuantHardness::Off,
            gbf_model::qat::QuantHardness::Off,
            gbf_model::qat::QuantHardness::Off,
        );
        block
    }

    fn bounded_kv_block() -> BoundedKvBlock {
        let mut block = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 2, 12).unwrap(),
            identity_norm(),
            activation(),
            identity_ternary(),
            identity_ternary(),
            identity_ternary(),
            activation(),
        )
        .unwrap();
        block.set_hardness(
            gbf_model::qat::QuantHardness::Off,
            gbf_model::qat::QuantHardness::Off,
            gbf_model::qat::QuantHardness::Off,
        );
        block
    }

    fn identity_norm() -> NormApproxQat {
        NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-8.0, 8.0).unwrap(),
            lut: gbf_model::qat::LutSpec::new(-1.0, 1.0, 3).unwrap(),
        })
    }

    fn activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
    }

    fn identity_ternary() -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(2, 2).unwrap(),
            vec![
                1.0, 0.0, //
                0.0, 1.0,
            ],
            None,
            vec![TernaryThreshold::new(0.5).unwrap(); 2],
            vec![Q8_8Scale::from_f32(1.0).unwrap(); 2],
        )
        .unwrap()
    }
}
