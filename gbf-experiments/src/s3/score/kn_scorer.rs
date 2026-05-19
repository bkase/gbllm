//! Kneser-Ney scorer adapter for the S3 bpc primitive.

use gbf_artifact::{CharId, TextCharSeq, VOCAB_SIZE};

use crate::s3::baseline::{
    KnBaselineProduct, KnConditionalModel, KnEffectiveCounts, p_kn_1, p_kn_2, p_kn_3, p_kn_4,
    p_kn_5,
};

use super::{
    Evaluator, EvaluatorOutput, ScoreError, ScorerKind, logits_from_probabilities,
    target_logprob_from_probabilities, train_hash,
};

/// S3 Kneser-Ney evaluator rebuilt from a fitted baseline report and train text.
#[derive(Debug, Clone)]
pub struct KnScorer<'a> {
    product: &'a KnBaselineProduct,
    model: KnConditionalModel,
}

impl<'a> KnScorer<'a> {
    /// Rebuild the runtime conditional model from the report's train split.
    pub fn from_product_and_train(
        product: &'a KnBaselineProduct,
        train_post: &TextCharSeq,
    ) -> Result<Self, ScoreError> {
        let observed = train_hash(train_post);
        if observed != product.train_post_sha256 {
            return Err(ScoreError::KnTrainHashMismatch {
                expected: product.train_post_sha256,
                observed,
            });
        }
        let counts = KnEffectiveCounts::fit(train_post)?;
        Ok(Self {
            product,
            model: KnConditionalModel::new(counts, product.discounts.clone()),
        })
    }

    /// Borrow the report used to configure this scorer.
    #[must_use]
    pub const fn product(&self) -> &KnBaselineProduct {
        self.product
    }

    fn probabilities(&self, prefix: &[CharId]) -> Result<Vec<f64>, ScoreError> {
        let order = (prefix.len() + 1).min(5);
        let mut probabilities = Vec::with_capacity(VOCAB_SIZE);
        for target in 0..VOCAB_SIZE {
            let target = target as CharId;
            let probability = match order {
                1 => p_kn_1(&self.model, target)?,
                2 => p_kn_2(&self.model, [prefix[prefix.len() - 1]], target)?,
                3 => p_kn_3(
                    &self.model,
                    [prefix[prefix.len() - 2], prefix[prefix.len() - 1]],
                    target,
                )?,
                4 => p_kn_4(
                    &self.model,
                    [
                        prefix[prefix.len() - 3],
                        prefix[prefix.len() - 2],
                        prefix[prefix.len() - 1],
                    ],
                    target,
                )?,
                5 => {
                    let suffix = &prefix[prefix.len() - 4..];
                    p_kn_5(
                        &self.model,
                        [suffix[0], suffix[1], suffix[2], suffix[3]],
                        target,
                    )?
                }
                _ => unreachable!("order is clamped to 1..=5"),
            };
            probabilities.push(probability.get());
        }
        Ok(probabilities)
    }
}

impl Evaluator for KnScorer<'_> {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::KnScorer
    }

    fn forward(&self, prefix: &[CharId], target_ix: usize) -> EvaluatorOutput {
        let probabilities = self
            .probabilities(prefix)
            .expect("KN probabilities compute");
        let logits = logits_from_probabilities(&probabilities);
        let target_logprob =
            target_logprob_from_probabilities(&probabilities, target_ix).expect("target has P>0");
        EvaluatorOutput {
            logits,
            target_logprob,
        }
    }

    fn reset_state(&mut self) {}
}
