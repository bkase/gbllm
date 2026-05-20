//! F-S5 emulator harness policy checks.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum H15FirstCommitCardinalityVerdict {
    Confirmed,
    Refuted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum H15FirstCommitCardinalityRefutation {
    EmptyFirstCommit,
    BatchedFirstCommit,
}

/// Policy-owned H15 report surface for first video-commit token cardinality.
///
/// This verifies only the D12/H15 cardinality invariant. The full emulator
/// harness still owns ticks, token/oracle agreement, and replay determinism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct H15FirstCommitCardinalityReport {
    pub first_commit_payload_len: u32,
    pub verdict: H15FirstCommitCardinalityVerdict,
    pub refutation: Option<H15FirstCommitCardinalityRefutation>,
}

#[must_use]
pub const fn verify_h15_first_commit_payload_len(
    first_commit_payload_len: u32,
) -> H15FirstCommitCardinalityReport {
    match first_commit_payload_len {
        0 => H15FirstCommitCardinalityReport {
            first_commit_payload_len,
            verdict: H15FirstCommitCardinalityVerdict::Refuted,
            refutation: Some(H15FirstCommitCardinalityRefutation::EmptyFirstCommit),
        },
        1 => H15FirstCommitCardinalityReport {
            first_commit_payload_len,
            verdict: H15FirstCommitCardinalityVerdict::Confirmed,
            refutation: None,
        },
        _ => H15FirstCommitCardinalityReport {
            first_commit_payload_len,
            verdict: H15FirstCommitCardinalityVerdict::Refuted,
            refutation: Some(H15FirstCommitCardinalityRefutation::BatchedFirstCommit),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h15_zero_token_first_commit_refutes() {
        assert_eq!(
            verify_h15_first_commit_payload_len(0),
            H15FirstCommitCardinalityReport {
                first_commit_payload_len: 0,
                verdict: H15FirstCommitCardinalityVerdict::Refuted,
                refutation: Some(H15FirstCommitCardinalityRefutation::EmptyFirstCommit),
            }
        );
    }

    #[test]
    fn h15_one_token_first_commit_confirms_cardinality() {
        assert_eq!(
            verify_h15_first_commit_payload_len(1),
            H15FirstCommitCardinalityReport {
                first_commit_payload_len: 1,
                verdict: H15FirstCommitCardinalityVerdict::Confirmed,
                refutation: None,
            }
        );
    }

    #[test]
    fn h15_two_token_first_commit_refutes_as_batch() {
        assert_eq!(
            verify_h15_first_commit_payload_len(2),
            H15FirstCommitCardinalityReport {
                first_commit_payload_len: 2,
                verdict: H15FirstCommitCardinalityVerdict::Refuted,
                refutation: Some(H15FirstCommitCardinalityRefutation::BatchedFirstCommit),
            }
        );
    }
}
