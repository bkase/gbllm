//! Model-facing sequence-state semantic contracts.

pub use gbf_artifact::sequence::{
    SequenceExportFacts, SequenceSemanticsError, SequenceSemanticsSpec, SequenceStateSize,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SequenceActivation {
    batch: usize,
    tokens: usize,
    d_model: usize,
    values: Vec<f32>,
}

impl SequenceActivation {
    pub fn new(
        batch: usize,
        tokens: usize,
        d_model: usize,
        values: Vec<f32>,
    ) -> Result<Self, SequenceActivationError> {
        if batch == 0 {
            return Err(SequenceActivationError::ZeroDim { field: "batch" });
        }
        if tokens == 0 {
            return Err(SequenceActivationError::ZeroDim { field: "tokens" });
        }
        if d_model == 0 {
            return Err(SequenceActivationError::ZeroDim { field: "d_model" });
        }
        let expected = batch
            .checked_mul(tokens)
            .and_then(|value| value.checked_mul(d_model))
            .ok_or(SequenceActivationError::ElementCountOverflow)?;
        if values.len() != expected {
            return Err(SequenceActivationError::ValueLenMismatch {
                expected,
                actual: values.len(),
            });
        }
        if let Some(index) = values.iter().position(|value| !value.is_finite()) {
            return Err(SequenceActivationError::NonFiniteValue { index });
        }

        Ok(Self {
            batch,
            tokens,
            d_model,
            values,
        })
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    pub fn tokens(&self) -> usize {
        self.tokens
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceState {
    spec: SequenceSemanticsSpec,
    bytes: Vec<u8>,
}

impl SequenceState {
    pub fn zeroed(spec: SequenceSemanticsSpec) -> Self {
        Self {
            spec,
            bytes: vec![0; spec.state_size().bytes_per_layer as usize],
        }
    }

    pub fn spec(&self) -> SequenceSemanticsSpec {
        self.spec
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceActivationError {
    ZeroDim { field: &'static str },
    ElementCountOverflow,
    ValueLenMismatch { expected: usize, actual: usize },
    NonFiniteValue { index: usize },
}

pub trait SequenceBlock {
    type Error;

    fn forward(
        &self,
        input: SequenceActivation,
        state: &mut SequenceState,
    ) -> Result<SequenceActivation, Self::Error>;
    fn state_init(&self) -> SequenceState;
    fn state_size(&self) -> SequenceStateSize;
    fn export_facts(&self) -> SequenceExportFacts;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    struct IdentitySequenceBlock {
        spec: SequenceSemanticsSpec,
    }

    impl SequenceBlock for IdentitySequenceBlock {
        type Error = core::convert::Infallible;

        fn forward(
            &self,
            input: SequenceActivation,
            state: &mut SequenceState,
        ) -> Result<SequenceActivation, Self::Error> {
            state.bytes_mut()[0] = 1;
            Ok(input)
        }

        fn state_init(&self) -> SequenceState {
            SequenceState::zeroed(self.spec)
        }

        fn state_size(&self) -> SequenceStateSize {
            self.spec.state_size()
        }

        fn export_facts(&self) -> SequenceExportFacts {
            SequenceExportFacts::for_spec(self.spec)
        }
    }

    #[test]
    fn sequence_block_trait_can_be_implemented_without_burn_dependency() {
        let block = IdentitySequenceBlock {
            spec: SequenceSemanticsSpec::linear_state(16).unwrap(),
        };
        let mut state = block.state_init();

        let input = SequenceActivation::new(1, 1, 2, vec![1.0, 2.0]).unwrap();
        let output = block.forward(input.clone(), &mut state).unwrap();
        let facts = block.export_facts();

        assert_eq!(output, input);
        assert_eq!(state.spec(), block.spec);
        assert_eq!(state.bytes()[0], 1);
        assert_eq!(
            block.state_size(),
            SequenceStateSize {
                bytes_per_layer: 16,
                bytes_per_token: 0,
                fixed_overhead: 0,
            }
        );
        assert_eq!(facts.spec(), block.spec);
        assert_eq!(facts.measured_state_size(), block.state_size());
    }

    #[test]
    fn sequence_activation_validates_shape_and_finiteness() {
        assert_eq!(
            SequenceActivation::new(0, 1, 1, vec![0.0]).unwrap_err(),
            SequenceActivationError::ZeroDim { field: "batch" }
        );
        assert_eq!(
            SequenceActivation::new(1, 2, 2, vec![0.0; 3]).unwrap_err(),
            SequenceActivationError::ValueLenMismatch {
                expected: 4,
                actual: 3,
            }
        );
        assert_eq!(
            SequenceActivation::new(1, 1, 1, vec![f32::NAN]).unwrap_err(),
            SequenceActivationError::NonFiniteValue { index: 0 }
        );
    }
}
