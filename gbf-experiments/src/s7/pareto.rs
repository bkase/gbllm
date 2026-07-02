//! S7 Pareto verdict helpers for the dense-vs-MoE comparison.

use std::fmt;

use gbf_artifact::{
    MatchedBytesPin, ParetoVerdict, S7DenseVsMoeParetoFields, S7FrontierParetoFields,
};

/// H3/H4 closure signals derived from the §11.3 Pareto verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S7ParetoClosureSignals {
    /// True when the Pareto branch also proves dense beat MoE on bpc.
    pub h3_refuted: bool,
    /// True when H4 did not confirm under strict dominance or byte-equivalent MoE win.
    pub h4_refuted: bool,
}

/// One point on the S7 `(median_val_bpc, deployed_bytes_total)` Pareto plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct S7ParetoPoint {
    /// Median validation bits-per-character for the topology.
    pub median_val_bpc: f64,
    /// Deployed byte total from the matched-bytes pin or frontier artifact.
    pub deployed_bytes_total: u64,
}

impl S7ParetoPoint {
    /// Construct a validated Pareto point.
    pub fn new(median_val_bpc: f64, deployed_bytes_total: u64) -> Result<Self, S7ParetoError> {
        validate_bpc("median_val_bpc", median_val_bpc)?;
        Ok(Self {
            median_val_bpc,
            deployed_bytes_total,
        })
    }
}

/// Compute the §11.3 S7 Pareto verdict.
///
/// Strict dominance uses exact deployed-byte inequalities. The D6 tolerance is
/// consulted only after strict dominance and exact ties have been ruled out,
/// where it decides the two byte-equivalence variants.
pub fn s7_pareto_verdict(
    moe: S7ParetoPoint,
    dense_matched: S7ParetoPoint,
    d6_tolerance_bytes: u64,
) -> Result<ParetoVerdict, S7ParetoError> {
    validate_bpc("moe.median_val_bpc", moe.median_val_bpc)?;
    validate_bpc("dense_matched.median_val_bpc", dense_matched.median_val_bpc)?;

    let bpc_le_moe = moe.median_val_bpc <= dense_matched.median_val_bpc;
    let bpc_le_dense = dense_matched.median_val_bpc <= moe.median_val_bpc;
    let by_le_moe = moe.deployed_bytes_total <= dense_matched.deployed_bytes_total;
    let by_le_dense = dense_matched.deployed_bytes_total <= moe.deployed_bytes_total;

    if bpc_le_moe
        && by_le_moe
        && (moe.median_val_bpc < dense_matched.median_val_bpc
            || moe.deployed_bytes_total < dense_matched.deployed_bytes_total)
    {
        return Ok(ParetoVerdict::MoeDominates);
    }

    if bpc_le_dense
        && by_le_dense
        && (dense_matched.median_val_bpc < moe.median_val_bpc
            || dense_matched.deployed_bytes_total < moe.deployed_bytes_total)
    {
        return Ok(ParetoVerdict::DenseDominates);
    }

    if moe.median_val_bpc == dense_matched.median_val_bpc
        && moe.deployed_bytes_total == dense_matched.deployed_bytes_total
    {
        return Ok(ParetoVerdict::Tied);
    }

    let bytes_equivalent = moe
        .deployed_bytes_total
        .abs_diff(dense_matched.deployed_bytes_total)
        <= d6_tolerance_bytes;

    if bytes_equivalent && moe.median_val_bpc < dense_matched.median_val_bpc {
        return Ok(ParetoVerdict::MoeWinsUnderByteEquivalence);
    }

    if bytes_equivalent && dense_matched.median_val_bpc < moe.median_val_bpc {
        return Ok(ParetoVerdict::DenseWinsUnderByteEquivalence);
    }

    Ok(ParetoVerdict::Incomparable)
}

/// Compute §11.3 from the deployed byte totals and tolerance pinned by D6.
pub fn s7_pareto_verdict_from_matched_bytes_pin(
    median_val_bpc_moe: f64,
    median_val_bpc_dense: f64,
    pin: &MatchedBytesPin,
) -> Result<ParetoVerdict, S7ParetoError> {
    s7_pareto_verdict(
        S7ParetoPoint::new(median_val_bpc_moe, pin.b_deployed_total_moe)?,
        S7ParetoPoint::new(median_val_bpc_dense, pin.b_deployed_total_dense)?,
        pin.tolerance_bytes,
    )
}

/// Return true iff the Pareto verdict confirms H4.
#[must_use]
pub const fn h4_confirmed_from_pareto(verdict: ParetoVerdict) -> bool {
    verdict.confirms_h4()
}

/// Derive the §11.3 closure signals used by §12 dispatch.
#[must_use]
pub const fn s7_pareto_closure_signals(verdict: ParetoVerdict) -> S7ParetoClosureSignals {
    let h3_refuted = matches!(
        verdict,
        ParetoVerdict::DenseDominates | ParetoVerdict::DenseWinsUnderByteEquivalence
    );
    S7ParetoClosureSignals {
        h3_refuted,
        h4_refuted: !verdict.confirms_h4(),
    }
}

/// Consume the `s7_dense_vs_moe.v1` Pareto field for H4 mapping.
#[must_use]
pub fn h4_confirmed_from_dense_vs_moe(report: &S7DenseVsMoeParetoFields) -> bool {
    report.pareto_verdict.confirms_h4()
}

/// Consume the `s7_frontier.v1` Pareto field for H4 mapping.
#[must_use]
pub fn h4_confirmed_from_frontier(report: &S7FrontierParetoFields) -> bool {
    report.pareto_verdict.confirms_h4()
}

/// Error returned by S7 Pareto helpers.
#[derive(Debug, Clone, PartialEq)]
pub enum S7ParetoError {
    /// A bpc value was NaN or infinite.
    NonFiniteBpc {
        /// Field that failed validation.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// A bpc value was negative.
    NegativeBpc {
        /// Field that failed validation.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
}

impl fmt::Display for S7ParetoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteBpc { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativeBpc { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
        }
    }
}

impl std::error::Error for S7ParetoError {}

fn validate_bpc(field: &'static str, value: f64) -> Result<(), S7ParetoError> {
    if !value.is_finite() {
        return Err(S7ParetoError::NonFiniteBpc { field, value });
    }
    if value < 0.0 {
        return Err(S7ParetoError::NegativeBpc { field, value });
    }
    Ok(())
}
