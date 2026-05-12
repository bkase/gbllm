//! S2 matched-protocol per-seed bpc gap primitives.

use std::error::Error;
use std::fmt;

use crate::S2_LOG_TARGET;
use crate::s2::schema::{GapBpc, S2BuildKind, S2ScoreReport};

/// Compute `s2_ternary_full.bpc - s2_fp_full.bpc` for five aligned seeds.
///
/// This convenience wrapper panics on invalid inputs. Use
/// [`try_gap_ternary_vs_fp`] when callers need to inspect the concrete error.
#[must_use]
pub fn gap_ternary_vs_fp(
    scores_t: &[S2ScoreReport; 5],
    scores_f: &[S2ScoreReport; 5],
) -> [GapBpc; 5] {
    try_gap_ternary_vs_fp(scores_t, scores_f)
        .expect("ternary-vs-fp score arrays must be seed-aligned and finite")
}

/// Checked form of [`gap_ternary_vs_fp`].
pub fn try_gap_ternary_vs_fp(
    scores_t: &[S2ScoreReport; 5],
    scores_f: &[S2ScoreReport; 5],
) -> Result<[GapBpc; 5], S2GapError> {
    gap_per_seed(
        scores_t,
        scores_f,
        GapSpec {
            left_build: S2BuildKind::s2_ternary_full,
            right_build: S2BuildKind::s2_fp_full,
            per_seed_event: "gap_ternary_vs_fp_per_seed",
            aggregate_build: "ternary_vs_fp",
        },
    )
}

/// Compute `s2_ternary_nodistill.bpc - s2_fp_full.bpc` for five aligned seeds.
///
/// This convenience wrapper panics on invalid inputs. Use
/// [`try_gap_nodistill_vs_fp`] when callers need to inspect the concrete error.
#[must_use]
pub fn gap_nodistill_vs_fp(
    scores_nd: &[S2ScoreReport; 5],
    scores_f: &[S2ScoreReport; 5],
) -> [GapBpc; 5] {
    try_gap_nodistill_vs_fp(scores_nd, scores_f)
        .expect("nodistill-vs-fp score arrays must be seed-aligned and finite")
}

/// Checked form of [`gap_nodistill_vs_fp`].
pub fn try_gap_nodistill_vs_fp(
    scores_nd: &[S2ScoreReport; 5],
    scores_f: &[S2ScoreReport; 5],
) -> Result<[GapBpc; 5], S2GapError> {
    gap_per_seed(
        scores_nd,
        scores_f,
        GapSpec {
            left_build: S2BuildKind::s2_ternary_nodistill,
            right_build: S2BuildKind::s2_fp_full,
            per_seed_event: "gap_nodistill_vs_fp_per_seed",
            aggregate_build: "nodistill_vs_fp",
        },
    )
}

#[derive(Debug, Clone, Copy)]
struct GapSpec {
    left_build: S2BuildKind,
    right_build: S2BuildKind,
    per_seed_event: &'static str,
    aggregate_build: &'static str,
}

fn gap_per_seed(
    left_scores: &[S2ScoreReport; 5],
    right_scores: &[S2ScoreReport; 5],
    spec: GapSpec,
) -> Result<[GapBpc; 5], S2GapError> {
    let mut gaps = [0.0_f64; 5];
    for index in 0..5 {
        let left = &left_scores[index];
        let right = &right_scores[index];
        ensure_build_kind(left, spec.left_build)?;
        ensure_build_kind(right, spec.right_build)?;
        if left.seed != right.seed {
            tracing::error!(
                target: S2_LOG_TARGET,
                event_name = "gap_seed_alignment_error",
                expected_seed = left.seed,
                got_seed = right.seed,
                "s2 gap seed alignment error"
            );
            return Err(S2GapError::SeedAlignment {
                index,
                expected_seed: left.seed,
                got_seed: right.seed,
            });
        }
        let gap = checked_gap(left, right)?;
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = spec.per_seed_event,
            seed = left.seed,
            gap,
            "s2 per-seed gap computed"
        );
        gaps[index] = gap;
    }
    emit_gap_aggregate(spec.aggregate_build, &gaps);
    Ok(gaps)
}

fn ensure_build_kind(score: &S2ScoreReport, expected: S2BuildKind) -> Result<(), S2GapError> {
    if score.build_kind == expected {
        return Ok(());
    }
    tracing::error!(
        target: S2_LOG_TARGET,
        event_name = "gap_build_kind_mismatch",
        seed = score.seed,
        expected = ?expected,
        got = ?score.build_kind,
        "s2 gap build-kind mismatch"
    );
    Err(S2GapError::BuildKindMismatch {
        seed: score.seed,
        expected,
        got: score.build_kind,
    })
}

fn checked_gap(left: &S2ScoreReport, right: &S2ScoreReport) -> Result<GapBpc, S2GapError> {
    if !left.bpc.is_finite() || !right.bpc.is_finite() {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "gap_non_finite",
            seed = left.seed,
            ternary_bpc = left.bpc,
            fp_bpc = right.bpc,
            "s2 gap non-finite bpc"
        );
        return Err(S2GapError::NonFiniteBpc {
            seed: left.seed,
            ternary_bpc: left.bpc,
            fp_bpc: right.bpc,
        });
    }
    let gap = left.bpc - right.bpc;
    if !gap.is_finite() {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "gap_non_finite",
            seed = left.seed,
            ternary_bpc = left.bpc,
            fp_bpc = right.bpc,
            "s2 gap non-finite result"
        );
        return Err(S2GapError::NonFiniteGap {
            seed: left.seed,
            ternary_bpc: left.bpc,
            fp_bpc: right.bpc,
        });
    }
    Ok(gap)
}

fn emit_gap_aggregate(build: &'static str, gaps: &[GapBpc; 5]) {
    let mut sorted = *gaps;
    sorted.sort_by(f64::total_cmp);
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "gap_aggregate",
        build,
        min = sorted[0],
        median = sorted[2],
        max = sorted[4],
        "s2 gap aggregate computed"
    );
}

/// Errors returned by the checked S2 gap primitives.
#[derive(Debug, Clone, PartialEq)]
pub enum S2GapError {
    /// Score arrays were not aligned by seed at the same index.
    SeedAlignment {
        /// Mismatched index.
        index: usize,
        /// Expected seed from the left-hand score array.
        expected_seed: u64,
        /// Observed seed from the fp comparator score array.
        got_seed: u64,
    },
    /// A score had the wrong build kind for the requested gap primitive.
    BuildKindMismatch {
        /// Seed containing the wrong build kind.
        seed: u64,
        /// Expected build kind.
        expected: S2BuildKind,
        /// Observed build kind.
        got: S2BuildKind,
    },
    /// A bpc input was not finite.
    NonFiniteBpc {
        /// Seed containing the invalid input.
        seed: u64,
        /// Left-hand ternary or nodistill bpc.
        ternary_bpc: f64,
        /// Right-hand fp bpc.
        fp_bpc: f64,
    },
    /// The resulting gap was not finite. This is defensive for manually
    /// mutated reports; schema-built reports only expose finite non-negative
    /// BPC values.
    NonFiniteGap {
        /// Seed containing the invalid result.
        seed: u64,
        /// Left-hand ternary or nodistill bpc.
        ternary_bpc: f64,
        /// Right-hand fp bpc.
        fp_bpc: f64,
    },
}

impl fmt::Display for S2GapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedAlignment {
                index,
                expected_seed,
                got_seed,
            } => write!(
                f,
                "gap score seed mismatch at index {index}: expected {expected_seed}, got {got_seed}"
            ),
            Self::BuildKindMismatch {
                seed,
                expected,
                got,
            } => write!(
                f,
                "seed {seed} has build kind {got:?}, expected {expected:?}"
            ),
            Self::NonFiniteBpc {
                seed,
                ternary_bpc,
                fp_bpc,
            } => write!(
                f,
                "seed {seed} has non-finite bpc input: ternary={ternary_bpc}, fp={fp_bpc}"
            ),
            Self::NonFiniteGap {
                seed,
                ternary_bpc,
                fp_bpc,
            } => write!(
                f,
                "seed {seed} produced non-finite gap: ternary={ternary_bpc}, fp={fp_bpc}"
            ),
        }
    }
}

impl Error for S2GapError {}
