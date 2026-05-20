//! F-S5 long-range repetition metric and H5 comparison helpers.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const LONG_RANGE_REPETITION_MIN_DISTANCE: usize = 64;
pub const H5_LONG_RANGE_CONFIRM_REDUCTION_PER_TOKEN: f64 = 0.10;
pub const H5_LONG_RANGE_REFUTE_REGRESSION_PER_TOKEN: f64 = 0.05;
pub const H5_VAL_BPC_REFUTE_REGRESSION: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongRangeRepetitionPenalty {
    pub generated_token_count: u64,
    pub repeated_pair_count: u64,
    pub pair_weighted_sum: f64,
    pub per_generated_token: f64,
}

/// Compute the non-negative F-S5 long-range repetition penalty.
///
/// The pair-weighted sum is `sum 1 / (j - i)` for repeated-token pairs at
/// least 64 generated-token positions apart. The reported penalty divides that
/// sum by the generated-token count.
#[must_use]
pub fn long_range_repetition_penalty<T: Ord>(generated_tokens: &[T]) -> LongRangeRepetitionPenalty {
    let mut positions_by_token: BTreeMap<&T, Vec<usize>> = BTreeMap::new();
    for (position, token) in generated_tokens.iter().enumerate() {
        positions_by_token.entry(token).or_default().push(position);
    }

    let mut repeated_pair_count = 0_u64;
    let mut pair_weighted_sum = 0.0_f64;

    for positions in positions_by_token.values() {
        for (left_index, left) in positions.iter().enumerate() {
            for right in &positions[left_index + 1..] {
                let distance = right - left;
                if distance >= LONG_RANGE_REPETITION_MIN_DISTANCE {
                    repeated_pair_count += 1;
                    pair_weighted_sum += 1.0 / distance as f64;
                }
            }
        }
    }

    let generated_token_count = generated_tokens.len() as u64;
    let per_generated_token = if generated_token_count == 0 {
        0.0
    } else {
        pair_weighted_sum / generated_token_count as f64
    };

    LongRangeRepetitionPenalty {
        generated_token_count,
        repeated_pair_count,
        pair_weighted_sum,
        per_generated_token,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct H5LongRangeEvidence {
    pub l_fix1_penalty_per_generated_token: f64,
    pub l_mt4_penalty_per_generated_token: f64,
    pub l_fix1_val_bpc_ternary: Option<f64>,
    pub l_mt4_val_bpc_ternary: Option<f64>,
}

impl H5LongRangeEvidence {
    #[must_use]
    pub const fn penalties_only(l_fix1: f64, l_mt4: f64) -> Self {
        // Penalty-only evidence is enough to refute a long-range regression,
        // but H5 confirmation additionally requires the val_bpc half.
        Self {
            l_fix1_penalty_per_generated_token: l_fix1,
            l_mt4_penalty_per_generated_token: l_mt4,
            l_fix1_val_bpc_ternary: None,
            l_mt4_val_bpc_ternary: None,
        }
    }

    #[must_use]
    pub const fn with_val_bpc(
        l_fix1_penalty: f64,
        l_mt4_penalty: f64,
        l_fix1_val_bpc: f64,
        l_mt4_val_bpc: f64,
    ) -> Self {
        Self {
            l_fix1_penalty_per_generated_token: l_fix1_penalty,
            l_mt4_penalty_per_generated_token: l_mt4_penalty,
            l_fix1_val_bpc_ternary: Some(l_fix1_val_bpc),
            l_mt4_val_bpc_ternary: Some(l_mt4_val_bpc),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum H5LongRangeVerdict {
    Confirmed,
    Refuted,
    NotConfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum H5LongRangeRefutation {
    LongRangePenaltyRegression,
    ValBpcRegression,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct H5LongRangeVerdictResult {
    pub verdict: H5LongRangeVerdict,
    pub penalty_reduction_per_generated_token: f64,
    pub val_bpc_delta_l_mt4_minus_l_fix1: Option<f64>,
    pub refutation: Option<H5LongRangeRefutation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum H5LongRangeError {
    NonFinitePenalty { field: &'static str, value: f64 },
    NegativePenalty { field: &'static str, value: f64 },
    NonFiniteValBpc { field: &'static str, value: f64 },
    IncompleteValBpcPair,
}

impl fmt::Display for H5LongRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinitePenalty { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativePenalty { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::NonFiniteValBpc { field, value } => {
                write!(f, "{field} must be finite when modeled, got {value}")
            }
            Self::IncompleteValBpcPair => {
                write!(
                    f,
                    "H5 val_bpc comparison must provide both variants or neither"
                )
            }
        }
    }
}

impl Error for H5LongRangeError {}

pub fn h5_long_range_verdict(
    evidence: H5LongRangeEvidence,
) -> Result<H5LongRangeVerdictResult, H5LongRangeError> {
    validate_penalty(
        "l_fix1_penalty_per_generated_token",
        evidence.l_fix1_penalty_per_generated_token,
    )?;
    validate_penalty(
        "l_mt4_penalty_per_generated_token",
        evidence.l_mt4_penalty_per_generated_token,
    )?;

    let val_bpc_delta_l_mt4_minus_l_fix1 = match (
        evidence.l_fix1_val_bpc_ternary,
        evidence.l_mt4_val_bpc_ternary,
    ) {
        (Some(l_fix1), Some(l_mt4)) => {
            validate_val_bpc("l_fix1_val_bpc_ternary", l_fix1)?;
            validate_val_bpc("l_mt4_val_bpc_ternary", l_mt4)?;
            Some(l_mt4 - l_fix1)
        }
        (None, None) => None,
        _ => return Err(H5LongRangeError::IncompleteValBpcPair),
    };

    let penalty_reduction_per_generated_token =
        evidence.l_fix1_penalty_per_generated_token - evidence.l_mt4_penalty_per_generated_token;

    if evidence.l_mt4_penalty_per_generated_token
        > evidence.l_fix1_penalty_per_generated_token + H5_LONG_RANGE_REFUTE_REGRESSION_PER_TOKEN
    {
        return Ok(H5LongRangeVerdictResult {
            verdict: H5LongRangeVerdict::Refuted,
            penalty_reduction_per_generated_token,
            val_bpc_delta_l_mt4_minus_l_fix1,
            refutation: Some(H5LongRangeRefutation::LongRangePenaltyRegression),
        });
    }

    if matches!(val_bpc_delta_l_mt4_minus_l_fix1, Some(delta) if delta > H5_VAL_BPC_REFUTE_REGRESSION)
    {
        return Ok(H5LongRangeVerdictResult {
            verdict: H5LongRangeVerdict::Refuted,
            penalty_reduction_per_generated_token,
            val_bpc_delta_l_mt4_minus_l_fix1,
            refutation: Some(H5LongRangeRefutation::ValBpcRegression),
        });
    }

    let val_bpc_non_worse = matches!(val_bpc_delta_l_mt4_minus_l_fix1, Some(delta) if delta <= 0.0);
    let verdict = if penalty_reduction_per_generated_token
        >= H5_LONG_RANGE_CONFIRM_REDUCTION_PER_TOKEN
        && val_bpc_non_worse
    {
        H5LongRangeVerdict::Confirmed
    } else {
        H5LongRangeVerdict::NotConfirmed
    };

    Ok(H5LongRangeVerdictResult {
        verdict,
        penalty_reduction_per_generated_token,
        val_bpc_delta_l_mt4_minus_l_fix1,
        refutation: None,
    })
}

fn validate_penalty(field: &'static str, value: f64) -> Result<(), H5LongRangeError> {
    if !value.is_finite() {
        return Err(H5LongRangeError::NonFinitePenalty { field, value });
    }
    if value < 0.0 {
        return Err(H5LongRangeError::NegativePenalty { field, value });
    }
    Ok(())
}

fn validate_val_bpc(field: &'static str, value: f64) -> Result<(), H5LongRangeError> {
    if !value.is_finite() {
        return Err(H5LongRangeError::NonFiniteValBpc { field, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1.0e-12;

    #[test]
    fn long_range_penalty_counts_one_pair_at_distance_64() {
        let mut tokens = vec![0_u16; 65];
        for token in &mut tokens[1..64] {
            *token = 1;
        }

        let penalty = long_range_repetition_penalty(&tokens);

        assert_eq!(penalty.generated_token_count, 65);
        assert_eq!(penalty.repeated_pair_count, 1);
        assert_close(penalty.pair_weighted_sum, 1.0 / 64.0);
        assert_close(penalty.per_generated_token, 1.0 / (64.0 * 65.0));
        assert!(penalty.per_generated_token >= 0.0);
    }

    #[test]
    fn long_range_penalty_counts_all_eligible_repeated_pairs() {
        let mut tokens = (0_u16..129_u16).collect::<Vec<_>>();
        tokens[0] = 999;
        tokens[64] = 999;
        tokens[128] = 999;

        let penalty = long_range_repetition_penalty(&tokens);
        let expected_pair_weighted_sum = 1.0 / 64.0 + 1.0 / 64.0 + 1.0 / 128.0;

        assert_eq!(penalty.generated_token_count, 129);
        assert_eq!(penalty.repeated_pair_count, 3);
        assert_close(penalty.pair_weighted_sum, expected_pair_weighted_sum);
        assert_close(
            penalty.per_generated_token,
            expected_pair_weighted_sum / 129.0,
        );
    }

    #[test]
    fn h5_long_range_confirmed_when_l_mt4_reduces_penalty_and_bpc_non_worse() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::with_val_bpc(0.42, 0.31, 1.25, 1.25))
                .unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::Confirmed);
        assert_close(result.penalty_reduction_per_generated_token, 0.11);
        assert_eq!(result.val_bpc_delta_l_mt4_minus_l_fix1, Some(0.0));
        assert_eq!(result.refutation, None);
    }

    #[test]
    fn h5_long_range_refuted_when_l_mt4_penalty_is_higher_by_sign_threshold() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::penalties_only(0.20, 0.251)).unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::Refuted);
        assert_eq!(
            result.refutation,
            Some(H5LongRangeRefutation::LongRangePenaltyRegression)
        );
        assert!(result.penalty_reduction_per_generated_token < 0.0);
    }

    #[test]
    fn h5_long_range_refuted_when_val_bpc_regresses_above_threshold() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::with_val_bpc(0.42, 0.31, 1.25, 1.301))
                .unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::Refuted);
        assert_eq!(
            result.refutation,
            Some(H5LongRangeRefutation::ValBpcRegression)
        );
        assert_close(result.val_bpc_delta_l_mt4_minus_l_fix1.unwrap(), 0.051);
    }

    #[test]
    fn h5_long_range_not_confirmed_when_penalty_reduction_is_too_small() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::with_val_bpc(0.42, 0.321, 1.25, 1.25))
                .unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::NotConfirmed);
        assert_eq!(result.refutation, None);
        assert_close(result.penalty_reduction_per_generated_token, 0.099);
    }

    #[test]
    fn h5_long_range_incomplete_val_bpc_pair_is_rejected() {
        assert_eq!(
            h5_long_range_verdict(H5LongRangeEvidence {
                l_fix1_penalty_per_generated_token: 0.42,
                l_mt4_penalty_per_generated_token: 0.31,
                l_fix1_val_bpc_ternary: Some(1.25),
                l_mt4_val_bpc_ternary: None,
            }),
            Err(H5LongRangeError::IncompleteValBpcPair)
        );
    }

    #[test]
    fn h5_long_range_val_bpc_regression_prevents_confirmed_before_refute_threshold() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::with_val_bpc(0.42, 0.31, 1.25, 1.26))
                .unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::NotConfirmed);
        assert_eq!(result.refutation, None);
        assert_close(result.val_bpc_delta_l_mt4_minus_l_fix1.unwrap(), 0.01);
    }

    #[test]
    fn h5_long_range_penalty_regression_boundary_is_strictly_greater_than() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::with_val_bpc(0.20, 0.25, 1.25, 1.25))
                .unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::NotConfirmed);
        assert_eq!(result.refutation, None);
        assert_close(result.penalty_reduction_per_generated_token, -0.05);
    }

    #[test]
    fn h5_long_range_penalties_only_cannot_confirm_h5_and_semantics() {
        let result =
            h5_long_range_verdict(H5LongRangeEvidence::penalties_only(0.42, 0.31)).unwrap();

        assert_eq!(result.verdict, H5LongRangeVerdict::NotConfirmed);
        assert_eq!(result.val_bpc_delta_l_mt4_minus_l_fix1, None);
        assert_eq!(result.refutation, None);
    }

    #[test]
    fn h5_long_range_rejects_negative_penalty() {
        assert_eq!(
            h5_long_range_verdict(H5LongRangeEvidence::penalties_only(-0.01, 0.0)),
            Err(H5LongRangeError::NegativePenalty {
                field: "l_fix1_penalty_per_generated_token",
                value: -0.01,
            })
        );
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "actual {actual} != expected {expected}"
        );
    }
}
