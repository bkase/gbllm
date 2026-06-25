//! S7 matched-bytes parity helpers.

pub use crate::s7::outcome::{AggregateParityVerdict, S7OutcomeError};

use crate::s7::outcome::aggregate_parity_verdict;

/// Compute §11.2 `s7_parity_aggregate`.
///
/// Matched-deployed-byte tolerance is checked before per-seed bpc parity so an
/// invalid byte comparison cannot be routed as a scientific parity failure.
pub fn s7_parity_aggregate(
    per_seed_passes: &[bool],
    bytes_diff: u64,
    d6_tolerance: u64,
) -> Result<AggregateParityVerdict, S7OutcomeError> {
    aggregate_parity_verdict(per_seed_passes, bytes_diff, d6_tolerance)
}
