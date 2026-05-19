//! Modified Kneser-Ney conditional probabilities.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::discounts::KnDiscounts;
use super::kn_effective_counts::KnEffectiveCounts;
use super::{BaselineError, KN_MAX_ORDER};

/// A finite non-negative f64 probability. There is intentionally no f32 constructor.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct BaselineProb(f64);

impl BaselineProb {
    /// Construct from f64 after finite/non-negative validation.
    pub fn try_new(value: f64) -> Result<Self, BaselineError> {
        if value.is_finite() && value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(BaselineError::InvalidProbability { value })
        }
    }

    /// Return the inner f64.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for BaselineProb {
    type Error = BaselineError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<BaselineProb> for f64 {
    fn from(value: BaselineProb) -> Self {
        value.0
    }
}

/// Fitted conditional distribution for P_KN_1..P_KN_5.
#[derive(Debug, Clone, PartialEq)]
pub struct KnConditionalModel {
    counts: KnEffectiveCounts,
    discounts: BTreeMap<u64, KnDiscounts>,
}

impl KnConditionalModel {
    /// Build from effective counts and per-order discounts.
    #[must_use]
    pub fn new(counts: KnEffectiveCounts, discounts: BTreeMap<u64, KnDiscounts>) -> Self {
        Self { counts, discounts }
    }

    /// Borrow effective counts.
    #[must_use]
    pub const fn counts(&self) -> &KnEffectiveCounts {
        &self.counts
    }

    /// Borrow fitted discounts.
    #[must_use]
    pub const fn discounts(&self) -> &BTreeMap<u64, KnDiscounts> {
        &self.discounts
    }

    /// Query P_KN_order(target | suffix(context)).
    pub fn probability(
        &self,
        order: usize,
        context: &[u8],
        target: u8,
    ) -> Result<BaselineProb, BaselineError> {
        if !(1..=KN_MAX_ORDER).contains(&order) {
            return Err(BaselineError::InvalidOrder { order });
        }
        let context_len = order.saturating_sub(1);
        let context = suffix(context, context_len);
        self.probability_inner(order, context, target)
    }

    /// Sum probability mass over the full S3 vocab for one context.
    pub fn probability_mass(&self, order: usize, context: &[u8]) -> Result<f64, BaselineError> {
        let mut sum = 0.0;
        for target in 0_u8..80 {
            sum += self.probability(order, context, target)?.get();
        }
        Ok(sum)
    }

    fn probability_inner(
        &self,
        order: usize,
        context: &[u8],
        target: u8,
    ) -> Result<BaselineProb, BaselineError> {
        if order == 1 {
            let denominator = self.counts.p1_denominator();
            let probability = if denominator == 0 {
                0.0
            } else {
                self.counts.p1_left_continuation_count(target) as f64 / denominator as f64
            };
            return BaselineProb::try_new(probability);
        }

        let context = suffix(context, order - 1);
        let context_total = self.counts.context_total(order, context)?;
        if context_total == 0 {
            return self.probability_inner(order - 1, suffix(context, order - 2), target);
        }

        let count = self.counts.count(order, context, target)?;
        let discounts = self
            .discounts
            .get(&(order as u64))
            .ok_or(BaselineError::InvalidOrder { order })?;
        let discount = discount_for_count(count, discounts);
        let discounted = ((count as f64) - discount).max(0.0) / context_total as f64;
        let gamma = self.gamma(order, context, context_total, discounts)?;
        let backoff = self
            .probability_inner(order - 1, suffix(context, order - 2), target)?
            .get();
        BaselineProb::try_new(discounted + gamma * backoff)
    }

    fn gamma(
        &self,
        order: usize,
        context: &[u8],
        context_total: u64,
        discounts: &KnDiscounts,
    ) -> Result<f64, BaselineError> {
        let (n1, n2, n3p) = self.counts.context_count_buckets(order, context)?;
        Ok(
            (discounts.d_1 * n1 as f64 + discounts.d_2 * n2 as f64 + discounts.d_3p * n3p as f64)
                / context_total as f64,
        )
    }
}

/// P_KN_1(w).
pub fn p_kn_1(model: &KnConditionalModel, target: u8) -> Result<BaselineProb, BaselineError> {
    model.probability(1, &[], target)
}

/// P_KN_2(w | h).
pub fn p_kn_2(
    model: &KnConditionalModel,
    context: [u8; 1],
    target: u8,
) -> Result<BaselineProb, BaselineError> {
    model.probability(2, &context, target)
}

/// P_KN_3(w | h).
pub fn p_kn_3(
    model: &KnConditionalModel,
    context: [u8; 2],
    target: u8,
) -> Result<BaselineProb, BaselineError> {
    model.probability(3, &context, target)
}

/// P_KN_4(w | h).
pub fn p_kn_4(
    model: &KnConditionalModel,
    context: [u8; 3],
    target: u8,
) -> Result<BaselineProb, BaselineError> {
    model.probability(4, &context, target)
}

/// P_KN_5(w | h).
pub fn p_kn_5(
    model: &KnConditionalModel,
    context: [u8; 4],
    target: u8,
) -> Result<BaselineProb, BaselineError> {
    model.probability(5, &context, target)
}

fn discount_for_count(count: u64, discounts: &KnDiscounts) -> f64 {
    match count {
        0 => 0.0,
        1 => discounts.d_1,
        2 => discounts.d_2,
        _ => discounts.d_3p,
    }
}

fn suffix(context: &[u8], len: usize) -> &[u8] {
    if context.len() <= len {
        context
    } else {
        &context[context.len() - len..]
    }
}
