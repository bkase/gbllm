//! Backend-independent input embedding and output classifier semantics.

use std::error::Error;
use std::fmt;

pub const BYTE_LEVEL_TIED_VOCAB_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingConfig {
    vocab_size: usize,
    d_model: usize,
    tie_mode: EmbeddingTieMode,
}

impl EmbeddingConfig {
    pub fn new(vocab_size: usize, d_model: usize) -> Result<Self, EmbeddingError> {
        let tie_mode = if vocab_size <= BYTE_LEVEL_TIED_VOCAB_LIMIT {
            EmbeddingTieMode::Tied
        } else {
            EmbeddingTieMode::Untied
        };
        Self::with_tie_mode(vocab_size, d_model, tie_mode)
    }

    pub fn tied(vocab_size: usize, d_model: usize) -> Result<Self, EmbeddingError> {
        Self::with_tie_mode(vocab_size, d_model, EmbeddingTieMode::Tied)
    }

    pub fn untied(vocab_size: usize, d_model: usize) -> Result<Self, EmbeddingError> {
        Self::with_tie_mode(vocab_size, d_model, EmbeddingTieMode::Untied)
    }

    pub fn with_tie_mode(
        vocab_size: usize,
        d_model: usize,
        tie_mode: EmbeddingTieMode,
    ) -> Result<Self, EmbeddingError> {
        validate_nonzero("vocab_size", vocab_size)?;
        validate_nonzero("d_model", d_model)?;
        let matrix_params = matrix_len(vocab_size, d_model)?;
        if tie_mode == EmbeddingTieMode::Untied {
            matrix_params
                .checked_mul(2)
                .ok_or(EmbeddingError::ShapeOverflow {
                    rows: vocab_size,
                    cols: d_model,
                })?;
        }

        Ok(Self {
            vocab_size,
            d_model,
            tie_mode,
        })
    }

    pub fn vocab_size(self) -> usize {
        self.vocab_size
    }

    pub fn d_model(self) -> usize {
        self.d_model
    }

    pub fn tie_mode(self) -> EmbeddingTieMode {
        self.tie_mode
    }

    pub fn tie_embeddings(self) -> bool {
        self.tie_mode == EmbeddingTieMode::Tied
    }

    pub fn parameter_count(self) -> usize {
        let matrix_params = self.vocab_size * self.d_model;
        match self.tie_mode {
            EmbeddingTieMode::Tied => matrix_params,
            EmbeddingTieMode::Untied => matrix_params * 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingTieMode {
    Tied,
    Untied,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingTied {
    vocab_size: usize,
    d_model: usize,
    weights: Vec<f32>,
}

impl EmbeddingTied {
    pub fn new(
        vocab_size: usize,
        d_model: usize,
        weights: Vec<f32>,
    ) -> Result<Self, EmbeddingError> {
        validate_weights("tied_embedding", vocab_size, d_model, &weights)?;
        Ok(Self {
            vocab_size,
            d_model,
            weights,
        })
    }

    pub fn from_config(config: EmbeddingConfig, weights: Vec<f32>) -> Result<Self, EmbeddingError> {
        if config.tie_mode() != EmbeddingTieMode::Tied {
            return Err(EmbeddingError::TieModeMismatch {
                expected: EmbeddingTieMode::Tied,
                actual: config.tie_mode(),
            });
        }
        Self::new(config.vocab_size(), config.d_model(), weights)
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn embedding_weights(&self) -> &[f32] {
        &self.weights
    }

    pub fn classifier_weights(&self) -> &[f32] {
        &self.weights
    }

    pub fn parameter_count(&self) -> usize {
        self.weights.len()
    }

    pub fn embed_one(&self, token_id: usize) -> Result<&[f32], EmbeddingError> {
        let row = self.row_range(token_id)?;
        Ok(&self.weights[row])
    }

    pub fn embed(&self, token_ids: &[usize]) -> Result<EmbeddingOutput, EmbeddingError> {
        embed_with_weights(self.vocab_size, self.d_model, &self.weights, token_ids)
    }

    pub fn classify(&self, hidden: &[f32]) -> Result<ClassifierOutput, EmbeddingError> {
        classify_with_weights(self.vocab_size, self.d_model, &self.weights, hidden)
    }

    fn row_range(&self, token_id: usize) -> Result<std::ops::Range<usize>, EmbeddingError> {
        row_range(token_id, self.vocab_size, self.d_model)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingUntied {
    vocab_size: usize,
    d_model: usize,
    embedding_weights: Vec<f32>,
    classifier_weights: Vec<f32>,
}

impl EmbeddingUntied {
    pub fn new(
        vocab_size: usize,
        d_model: usize,
        embedding_weights: Vec<f32>,
        classifier_weights: Vec<f32>,
    ) -> Result<Self, EmbeddingError> {
        validate_weights("input_embedding", vocab_size, d_model, &embedding_weights)?;
        validate_weights(
            "output_classifier",
            vocab_size,
            d_model,
            &classifier_weights,
        )?;
        Ok(Self {
            vocab_size,
            d_model,
            embedding_weights,
            classifier_weights,
        })
    }

    pub fn from_config(
        config: EmbeddingConfig,
        embedding_weights: Vec<f32>,
        classifier_weights: Vec<f32>,
    ) -> Result<Self, EmbeddingError> {
        if config.tie_mode() != EmbeddingTieMode::Untied {
            return Err(EmbeddingError::TieModeMismatch {
                expected: EmbeddingTieMode::Untied,
                actual: config.tie_mode(),
            });
        }
        Self::new(
            config.vocab_size(),
            config.d_model(),
            embedding_weights,
            classifier_weights,
        )
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn embedding_weights(&self) -> &[f32] {
        &self.embedding_weights
    }

    pub fn classifier_weights(&self) -> &[f32] {
        &self.classifier_weights
    }

    pub fn parameter_count(&self) -> usize {
        self.embedding_weights.len() + self.classifier_weights.len()
    }

    pub fn embed_one(&self, token_id: usize) -> Result<&[f32], EmbeddingError> {
        let row = row_range(token_id, self.vocab_size, self.d_model)?;
        Ok(&self.embedding_weights[row])
    }

    pub fn embed(&self, token_ids: &[usize]) -> Result<EmbeddingOutput, EmbeddingError> {
        embed_with_weights(
            self.vocab_size,
            self.d_model,
            &self.embedding_weights,
            token_ids,
        )
    }

    pub fn classify(&self, hidden: &[f32]) -> Result<ClassifierOutput, EmbeddingError> {
        classify_with_weights(
            self.vocab_size,
            self.d_model,
            &self.classifier_weights,
            hidden,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingOutput {
    token_count: usize,
    d_model: usize,
    values: Vec<f32>,
}

impl EmbeddingOutput {
    pub fn token_count(&self) -> usize {
        self.token_count
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn shape(&self) -> [usize; 2] {
        [self.token_count, self.d_model]
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassifierOutput {
    batch_size: usize,
    vocab_size: usize,
    values: Vec<f32>,
}

impl ClassifierOutput {
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    pub fn shape(&self) -> [usize; 2] {
        [self.batch_size, self.vocab_size]
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingError {
    EmptyDimension {
        field: &'static str,
    },
    ShapeOverflow {
        rows: usize,
        cols: usize,
    },
    WeightLenMismatch {
        matrix: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteWeight {
        matrix: &'static str,
        index: usize,
    },
    TokenIdOutOfRange {
        token_id: usize,
        vocab_size: usize,
    },
    HiddenDimMismatch {
        expected_multiple: usize,
        actual: usize,
    },
    NonFiniteHidden {
        index: usize,
    },
    TieModeMismatch {
        expected: EmbeddingTieMode,
        actual: EmbeddingTieMode,
    },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDimension { field } => write!(f, "{field} must be nonzero"),
            Self::ShapeOverflow { rows, cols } => {
                write!(f, "embedding matrix shape {rows}x{cols} overflows")
            }
            Self::WeightLenMismatch {
                matrix,
                expected,
                actual,
            } => write!(
                f,
                "{matrix} weight length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteWeight { matrix, index } => {
                write!(f, "{matrix} weight at index {index} is not finite")
            }
            Self::TokenIdOutOfRange {
                token_id,
                vocab_size,
            } => write!(
                f,
                "token id {token_id} is out of range for vocab size {vocab_size}"
            ),
            Self::HiddenDimMismatch {
                expected_multiple,
                actual,
            } => write!(
                f,
                "hidden length {actual} is not a multiple of d_model {expected_multiple}"
            ),
            Self::NonFiniteHidden { index } => {
                write!(f, "hidden activation at index {index} is not finite")
            }
            Self::TieModeMismatch { expected, actual } => {
                write!(
                    f,
                    "embedding tie mode mismatch: expected {expected:?}, got {actual:?}"
                )
            }
        }
    }
}

impl Error for EmbeddingError {}

fn embed_with_weights(
    vocab_size: usize,
    d_model: usize,
    weights: &[f32],
    token_ids: &[usize],
) -> Result<EmbeddingOutput, EmbeddingError> {
    let output_len = token_ids
        .len()
        .checked_mul(d_model)
        .ok_or(EmbeddingError::ShapeOverflow {
            rows: token_ids.len(),
            cols: d_model,
        })?;
    let mut values = Vec::with_capacity(output_len);
    for &token_id in token_ids {
        let row = row_range(token_id, vocab_size, d_model)?;
        values.extend_from_slice(&weights[row]);
    }
    Ok(EmbeddingOutput {
        token_count: token_ids.len(),
        d_model,
        values,
    })
}

fn classify_with_weights(
    vocab_size: usize,
    d_model: usize,
    classifier_weights: &[f32],
    hidden: &[f32],
) -> Result<ClassifierOutput, EmbeddingError> {
    validate_hidden(d_model, hidden)?;
    let batch_size = hidden.len() / d_model;
    let output_len = batch_size
        .checked_mul(vocab_size)
        .ok_or(EmbeddingError::ShapeOverflow {
            rows: batch_size,
            cols: vocab_size,
        })?;
    let mut values = Vec::with_capacity(output_len);

    for hidden_row in hidden.chunks_exact(d_model) {
        for token_id in 0..vocab_size {
            let classifier_row = row_range(token_id, vocab_size, d_model)?;
            let logit = hidden_row
                .iter()
                .zip(&classifier_weights[classifier_row])
                .map(|(hidden_value, weight)| hidden_value * weight)
                .sum();
            values.push(logit);
        }
    }

    Ok(ClassifierOutput {
        batch_size,
        vocab_size,
        values,
    })
}

fn validate_weights(
    matrix: &'static str,
    vocab_size: usize,
    d_model: usize,
    weights: &[f32],
) -> Result<(), EmbeddingError> {
    validate_nonzero("vocab_size", vocab_size)?;
    validate_nonzero("d_model", d_model)?;
    let expected = matrix_len(vocab_size, d_model)?;
    if weights.len() != expected {
        return Err(EmbeddingError::WeightLenMismatch {
            matrix,
            expected,
            actual: weights.len(),
        });
    }
    validate_finite_weights(matrix, weights)
}

fn validate_nonzero(field: &'static str, value: usize) -> Result<(), EmbeddingError> {
    if value == 0 {
        return Err(EmbeddingError::EmptyDimension { field });
    }
    Ok(())
}

fn validate_hidden(d_model: usize, hidden: &[f32]) -> Result<(), EmbeddingError> {
    if !hidden.len().is_multiple_of(d_model) {
        return Err(EmbeddingError::HiddenDimMismatch {
            expected_multiple: d_model,
            actual: hidden.len(),
        });
    }
    for (index, value) in hidden.iter().enumerate() {
        if !value.is_finite() {
            return Err(EmbeddingError::NonFiniteHidden { index });
        }
    }
    Ok(())
}

fn validate_finite_weights(matrix: &'static str, weights: &[f32]) -> Result<(), EmbeddingError> {
    for (index, value) in weights.iter().enumerate() {
        if !value.is_finite() {
            return Err(EmbeddingError::NonFiniteWeight { matrix, index });
        }
    }
    Ok(())
}

fn matrix_len(rows: usize, cols: usize) -> Result<usize, EmbeddingError> {
    rows.checked_mul(cols)
        .ok_or(EmbeddingError::ShapeOverflow { rows, cols })
}

fn row_range(
    token_id: usize,
    vocab_size: usize,
    d_model: usize,
) -> Result<std::ops::Range<usize>, EmbeddingError> {
    if token_id >= vocab_size {
        return Err(EmbeddingError::TokenIdOutOfRange {
            token_id,
            vocab_size,
        });
    }
    let start = token_id * d_model;
    Ok(start..start + d_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_config_defaults_to_tied_for_byte_vocab_and_untied_for_large_vocab() {
        assert_eq!(
            EmbeddingConfig::new(256, 8).unwrap().tie_mode(),
            EmbeddingTieMode::Tied
        );
        assert_eq!(
            EmbeddingConfig::new(257, 8).unwrap().tie_mode(),
            EmbeddingTieMode::Untied
        );

        let untied_byte_config = EmbeddingConfig::untied(128, 8).unwrap();
        assert_eq!(untied_byte_config.tie_mode(), EmbeddingTieMode::Untied);
        assert!(!untied_byte_config.tie_embeddings());
        assert_eq!(
            EmbeddingConfig::tied(128, 8).unwrap().parameter_count(),
            1024
        );
        assert_eq!(untied_byte_config.parameter_count(), 2048);

        assert_eq!(
            EmbeddingConfig::new(0, 8),
            Err(EmbeddingError::EmptyDimension {
                field: "vocab_size"
            })
        );
        assert_eq!(
            EmbeddingConfig::new(8, 0),
            Err(EmbeddingError::EmptyDimension { field: "d_model" })
        );
    }

    #[test]
    fn embedding_tied_reuses_one_weight_matrix_for_embed_and_classify() {
        let layer = EmbeddingTied::new(
            3,
            2,
            vec![
                1.0, 2.0, //
                3.0, 4.0, //
                -1.0, 0.5,
            ],
        )
        .unwrap();

        assert!(std::ptr::eq(
            layer.embedding_weights().as_ptr(),
            layer.classifier_weights().as_ptr()
        ));
        assert_eq!(layer.parameter_count(), 6);
        assert_eq!(layer.embed_one(1).unwrap(), &[3.0, 4.0]);

        let embeddings = layer.embed(&[2, 0]).unwrap();
        assert_eq!(embeddings.shape(), [2, 2]);
        assert_eq!(embeddings.values(), &[-1.0, 0.5, 1.0, 2.0]);

        let logits = layer.classify(&[1.0, 1.0, 2.0, 0.0]).unwrap();
        assert_eq!(logits.shape(), [2, 3]);
        assert_eq!(logits.values(), &[3.0, 7.0, -0.5, 2.0, 6.0, -2.0]);
    }

    #[test]
    fn embedding_untied_uses_separate_embedding_and_classifier_matrices() {
        let layer = EmbeddingUntied::new(
            3,
            2,
            vec![
                10.0, 20.0, //
                30.0, 40.0, //
                50.0, 60.0,
            ],
            vec![
                1.0, 0.0, //
                0.0, 1.0, //
                1.0, 1.0,
            ],
        )
        .unwrap();

        assert!(!std::ptr::eq(
            layer.embedding_weights().as_ptr(),
            layer.classifier_weights().as_ptr()
        ));
        assert_eq!(layer.parameter_count(), 12);
        assert_eq!(layer.embed_one(2).unwrap(), &[50.0, 60.0]);

        let embeddings = layer.embed(&[0, 2]).unwrap();
        assert_eq!(embeddings.shape(), [2, 2]);
        assert_eq!(embeddings.values(), &[10.0, 20.0, 50.0, 60.0]);

        let logits = layer.classify(&[2.0, 3.0]).unwrap();
        assert_eq!(logits.shape(), [1, 3]);
        assert_eq!(logits.values(), &[2.0, 3.0, 5.0]);
    }

    #[test]
    fn embedding_from_default_config_constructs_expected_tie_mode() {
        let tied_config = EmbeddingConfig::new(3, 2).unwrap();
        let tied = EmbeddingTied::from_config(tied_config, vec![0.0; 6]).unwrap();
        assert_eq!(tied.vocab_size(), 3);
        assert_eq!(tied.d_model(), 2);
        assert_eq!(tied.parameter_count(), 6);

        let untied_config = EmbeddingConfig::new(257, 2).unwrap();
        let untied =
            EmbeddingUntied::from_config(untied_config, vec![0.0; 514], vec![1.0; 514]).unwrap();
        assert_eq!(untied.vocab_size(), 257);
        assert_eq!(untied.d_model(), 2);
        assert_eq!(untied.parameter_count(), 1028);
    }

    #[test]
    fn embedding_from_config_rejects_wrong_tie_mode() {
        let tied_config = EmbeddingConfig::with_tie_mode(3, 2, EmbeddingTieMode::Tied).unwrap();
        let untied_config = EmbeddingConfig::with_tie_mode(3, 2, EmbeddingTieMode::Untied).unwrap();

        assert_eq!(
            EmbeddingTied::from_config(untied_config, vec![0.0; 6]),
            Err(EmbeddingError::TieModeMismatch {
                expected: EmbeddingTieMode::Tied,
                actual: EmbeddingTieMode::Untied,
            })
        );
        assert_eq!(
            EmbeddingUntied::from_config(tied_config, vec![0.0; 6], vec![0.0; 6]),
            Err(EmbeddingError::TieModeMismatch {
                expected: EmbeddingTieMode::Untied,
                actual: EmbeddingTieMode::Tied,
            })
        );
    }

    #[test]
    fn embedding_rejects_invalid_weight_and_input_contracts() {
        assert_eq!(
            EmbeddingTied::new(2, 2, vec![1.0, 2.0, 3.0]),
            Err(EmbeddingError::WeightLenMismatch {
                matrix: "tied_embedding",
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(
            EmbeddingUntied::new(2, 2, vec![1.0, f32::INFINITY, 3.0, 4.0], vec![0.0; 4]),
            Err(EmbeddingError::NonFiniteWeight {
                matrix: "input_embedding",
                index: 1,
            })
        );

        let layer = EmbeddingTied::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(
            layer.embed(&[0, 2]),
            Err(EmbeddingError::TokenIdOutOfRange {
                token_id: 2,
                vocab_size: 2,
            })
        );
        assert_eq!(
            layer.classify(&[1.0, 2.0, 3.0]),
            Err(EmbeddingError::HiddenDimMismatch {
                expected_multiple: 2,
                actual: 3,
            })
        );
        assert_eq!(
            layer.classify(&[1.0, f32::NAN]),
            Err(EmbeddingError::NonFiniteHidden { index: 1 })
        );
    }
}
