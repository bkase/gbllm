//! Burn-backed embedding/classifier adapter.

use std::error::Error;
use std::fmt;

use gbf_model::embeddings::{EmbeddingError, EmbeddingTied, EmbeddingUntied};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam, burn_linear,
    float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape,
};

#[derive(BurnModule, Debug)]
pub struct EmbeddingTiedBurn<B: BurnBackend> {
    #[module(skip)]
    vocab_size: usize,
    #[module(skip)]
    d_model: usize,
    weights: BurnParam<BurnFloatTensor<B, 2>>,
}

impl<B: BurnBackend> EmbeddingTiedBurn<B> {
    pub fn from_core(
        core: EmbeddingTied,
        device: &BurnDevice<B>,
    ) -> Result<Self, EmbeddingBurnError> {
        let vocab_size = core.vocab_size();
        let d_model = core.d_model();
        Ok(Self {
            vocab_size,
            d_model,
            weights: BurnParam::from_tensor(float_tensor_from_vec(
                core.embedding_weights().to_vec(),
                [vocab_size, d_model],
                device,
            )?),
        })
    }

    #[must_use]
    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    #[must_use]
    pub fn d_model(&self) -> usize {
        self.d_model
    }

    #[must_use]
    pub fn embedding_weights(&self) -> BurnFloatTensor<B, 2> {
        self.weights.val()
    }

    #[must_use]
    pub fn classifier_weights(&self) -> BurnFloatTensor<B, 2> {
        self.weights.val()
    }

    pub fn embed_one(
        &self,
        token_id: usize,
        device: &BurnDevice<B>,
    ) -> Result<BurnFloatTensor<B, 1>, EmbeddingBurnError> {
        let token_distribution = one_hot(token_id, self.vocab_size, device)?;
        self.embed_distribution(token_distribution)
    }

    pub fn embed_distribution(
        &self,
        token_distribution: BurnFloatTensor<B, 1>,
    ) -> Result<BurnFloatTensor<B, 1>, EmbeddingBurnError> {
        validate_last_dim("token_distribution", self.vocab_size, &token_distribution)?;
        Ok(burn_linear(
            token_distribution,
            self.embedding_weights(),
            None::<BurnFloatTensor<B, 1>>,
        ))
    }

    pub fn classify<const D: usize>(
        &self,
        hidden: BurnFloatTensor<B, D>,
    ) -> Result<BurnFloatTensor<B, D>, EmbeddingBurnError> {
        validate_last_dim("hidden", self.d_model, &hidden)?;
        Ok(burn_linear(
            hidden,
            self.classifier_weights().transpose(),
            None::<BurnFloatTensor<B, 1>>,
        ))
    }

    pub fn to_core_from_trained_state(&self) -> Result<EmbeddingTied, EmbeddingBurnError> {
        let weights = float_tensor_into_vec(self.embedding_weights().detach())?;
        EmbeddingTied::new(self.vocab_size, self.d_model, weights)
            .map_err(EmbeddingBurnError::Model)
    }
}

#[derive(BurnModule, Debug)]
pub struct EmbeddingUntiedBurn<B: BurnBackend> {
    #[module(skip)]
    vocab_size: usize,
    #[module(skip)]
    d_model: usize,
    embedding_weights: BurnParam<BurnFloatTensor<B, 2>>,
    classifier_weights: BurnParam<BurnFloatTensor<B, 2>>,
}

impl<B: BurnBackend> EmbeddingUntiedBurn<B> {
    pub fn from_core(
        core: EmbeddingUntied,
        device: &BurnDevice<B>,
    ) -> Result<Self, EmbeddingBurnError> {
        let vocab_size = core.vocab_size();
        let d_model = core.d_model();
        Ok(Self {
            vocab_size,
            d_model,
            embedding_weights: BurnParam::from_tensor(float_tensor_from_vec(
                core.embedding_weights().to_vec(),
                [vocab_size, d_model],
                device,
            )?),
            classifier_weights: BurnParam::from_tensor(float_tensor_from_vec(
                core.classifier_weights().to_vec(),
                [vocab_size, d_model],
                device,
            )?),
        })
    }

    #[must_use]
    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    #[must_use]
    pub fn d_model(&self) -> usize {
        self.d_model
    }

    #[must_use]
    pub fn embedding_weights(&self) -> BurnFloatTensor<B, 2> {
        self.embedding_weights.val()
    }

    #[must_use]
    pub fn classifier_weights(&self) -> BurnFloatTensor<B, 2> {
        self.classifier_weights.val()
    }

    pub fn embed_one(
        &self,
        token_id: usize,
        device: &BurnDevice<B>,
    ) -> Result<BurnFloatTensor<B, 1>, EmbeddingBurnError> {
        let token_distribution = one_hot(token_id, self.vocab_size, device)?;
        self.embed_distribution(token_distribution)
    }

    pub fn embed_distribution(
        &self,
        token_distribution: BurnFloatTensor<B, 1>,
    ) -> Result<BurnFloatTensor<B, 1>, EmbeddingBurnError> {
        validate_last_dim("token_distribution", self.vocab_size, &token_distribution)?;
        Ok(burn_linear(
            token_distribution,
            self.embedding_weights(),
            None::<BurnFloatTensor<B, 1>>,
        ))
    }

    pub fn classify<const D: usize>(
        &self,
        hidden: BurnFloatTensor<B, D>,
    ) -> Result<BurnFloatTensor<B, D>, EmbeddingBurnError> {
        validate_last_dim("hidden", self.d_model, &hidden)?;
        Ok(burn_linear(
            hidden,
            self.classifier_weights().transpose(),
            None::<BurnFloatTensor<B, 1>>,
        ))
    }

    pub fn to_core_from_trained_state(&self) -> Result<EmbeddingUntied, EmbeddingBurnError> {
        let embedding_weights = float_tensor_into_vec(self.embedding_weights().detach())?;
        let classifier_weights = float_tensor_into_vec(self.classifier_weights().detach())?;
        EmbeddingUntied::new(
            self.vocab_size,
            self.d_model,
            embedding_weights,
            classifier_weights,
        )
        .map_err(EmbeddingBurnError::Model)
    }
}

#[derive(Debug)]
pub enum EmbeddingBurnError {
    Adapter(BurnAdapterError),
    Model(EmbeddingError),
    TokenIdOutOfRange {
        token_id: usize,
        vocab_size: usize,
    },
    LastDimMismatch {
        name: &'static str,
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
}

impl fmt::Display for EmbeddingBurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::TokenIdOutOfRange {
                token_id,
                vocab_size,
            } => write!(
                f,
                "token id {token_id} is out of range for vocab size {vocab_size}"
            ),
            Self::LastDimMismatch {
                name,
                expected,
                actual,
                shape,
            } => write!(
                f,
                "{name} last dimension mismatch: expected {expected}, got {actual} for shape {shape:?}"
            ),
        }
    }
}

impl Error for EmbeddingBurnError {}

impl From<BurnAdapterError> for EmbeddingBurnError {
    fn from(error: BurnAdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<EmbeddingError> for EmbeddingBurnError {
    fn from(error: EmbeddingError) -> Self {
        Self::Model(error)
    }
}

fn one_hot<B: BurnBackend>(
    token_id: usize,
    vocab_size: usize,
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, EmbeddingBurnError> {
    if token_id >= vocab_size {
        return Err(EmbeddingBurnError::TokenIdOutOfRange {
            token_id,
            vocab_size,
        });
    }

    let mut values = vec![0.0; vocab_size];
    values[token_id] = 1.0;
    float_tensor_from_vec(values, [vocab_size], device).map_err(Into::into)
}

fn validate_last_dim<B: BurnBackend, const D: usize>(
    name: &'static str,
    expected: usize,
    tensor: &BurnFloatTensor<B, D>,
) -> Result<(), EmbeddingBurnError> {
    let shape = float_tensor_shape(tensor);
    let actual = *shape
        .last()
        .expect("Burn tensors always carry a rank in their type");
    if actual != expected {
        return Err(EmbeddingBurnError::LastDimMismatch {
            name,
            expected,
            actual,
            shape: shape.to_vec(),
        });
    }

    Ok(())
}
