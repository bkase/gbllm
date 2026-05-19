//! S3 TinyStories 5-gram Kneser-Ney baseline helpers.

use std::fmt;

use gbf_foundation::CanonicalJsonError;

pub mod canonical_counts_write;
pub mod conditional;
pub mod discounts;
pub mod kn_effective_counts;
pub mod operation;

pub use canonical_counts_write::CanonicalKnCountsWrite;
pub use conditional::{BaselineProb, KnConditionalModel, p_kn_1, p_kn_2, p_kn_3, p_kn_4, p_kn_5};
pub use discounts::{KnDiscounts, fit_discounts, fit_discounts_for_order};
pub use kn_effective_counts::{
    KnEffectiveCounts, NgramKey, c_continuation_counts, c5_raw_counts, count_of_counts,
};
pub use operation::{CountsSummary, KnBaselineInputs, KnBaselineProduct, s3_fit_kn5};

/// Maximum order pinned by F-S3 D4.
pub const KN_MAX_ORDER: usize = 5;

/// Validation reset chunk size in normalized characters.
pub const KN_RESET_CHUNK_SIZE: usize = 128;

/// S3 baseline tracing target.
pub const S3_BASELINE_LOG_TARGET: &str = "gbf_experiments::s3::baseline";

/// Errors from the S3 modified Kneser-Ney baseline.
#[derive(Debug)]
pub enum BaselineError {
    /// Training character sequence must contain at least five characters.
    TrainTooShort {
        /// Minimum required character count.
        min: usize,
        /// Observed character count.
        observed: usize,
    },
    /// Validation character sequence must be non-empty.
    EmptyValidation,
    /// A caller requested an order outside the pinned KN range.
    InvalidOrder {
        /// Observed or requested order.
        order: usize,
    },
    /// Discount D-rule is undefined because one or more count-of-counts is zero.
    DiscountPreconditionsViolated {
        /// Effective order whose count-of-counts failed.
        order: u64,
        /// Missing count buckets from {1, 2, 3}.
        missing: Vec<u64>,
    },
    /// A fitted discount fell outside its RFC-bounded range.
    DiscountOutOfBounds {
        /// Effective order whose discount was checked.
        order: u64,
        /// Discount field name.
        field: &'static str,
        /// Observed discount value.
        value: f64,
        /// Inclusive lower bound, except `y_k` where the bound is strict.
        lower: f64,
        /// Inclusive upper bound, except `y_k` where the bound is strict.
        upper: f64,
    },
    /// Probability construction requires a finite non-negative f64.
    InvalidProbability {
        /// Observed invalid probability value.
        value: f64,
    },
    /// A scored validation target received zero probability.
    ZeroProbability {
        /// Order used for the failed query.
        order: usize,
        /// Offset within the reset chunk.
        chunk_offset: usize,
        /// Target token id.
        target: u8,
    },
    /// Canonical JSON or DomainHash construction failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrainTooShort { min, observed } => {
                write!(
                    f,
                    "S3 KN baseline requires at least {min} train chars, observed {observed}"
                )
            }
            Self::EmptyValidation => f.write_str("S3 KN baseline requires non-empty validation"),
            Self::InvalidOrder { order } => write!(f, "invalid S3 KN order {order}"),
            Self::DiscountPreconditionsViolated { order, missing } => write!(
                f,
                "S3 KN discount preconditions violated for order {order}; missing count-of-counts {missing:?}"
            ),
            Self::DiscountOutOfBounds {
                order,
                field,
                value,
                lower,
                upper,
            } => write!(
                f,
                "S3 KN discount {field} for order {order} out of bounds: {value} not in [{lower}, {upper}]"
            ),
            Self::InvalidProbability { value } => {
                write!(
                    f,
                    "S3 KN probability must be finite and non-negative, got {value}"
                )
            }
            Self::ZeroProbability {
                order,
                chunk_offset,
                target,
            } => write!(
                f,
                "S3 KN order {order} assigned zero probability to target {target} at chunk offset {chunk_offset}"
            ),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BaselineError {}

impl From<CanonicalJsonError> for BaselineError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
