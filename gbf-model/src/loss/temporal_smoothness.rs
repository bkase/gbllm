//! Pair-set semantics for S7 temporal smoothness regularization.
//!
//! This module owns the backend-independent `pairs(b)` contract for
//! `s7_temporal_smoothness`. Tensor math and Burn autodiff remain in
//! `gbf-train`; the pair set here is the source of truth those paths consume.

use std::error::Error;
use std::fmt;

/// S7's pinned D10 temporal smoothness window.
pub const S7_DEFAULT_SMOOTHNESS_WINDOW: u16 = 32;

/// Constructor-validated temporal smoothness window.
///
/// Window size one is intentionally rejected by S7 D10: it is mathematically
/// valid as an adjacent-token penalty, but too weak for the S7 full-window
/// temporal smoothness claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SmoothnessWindow(u16);

impl SmoothnessWindow {
    pub fn new(value: u16) -> Result<Self, TemporalSmoothnessError> {
        if value < 2 {
            return Err(TemporalSmoothnessError::SmoothnessWindowTooSmall { value });
        }

        Ok(Self(value))
    }

    pub fn s7_default() -> Self {
        Self(S7_DEFAULT_SMOOTHNESS_WINDOW)
    }

    pub fn get(self) -> u16 {
        self.0
    }

    fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

/// One `(t, u)` entry in the S7 `pairs(b)` set.
///
/// `t` is the current token index and `u` is a previous token index inside the
/// validated smoothness window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemporalSmoothnessPair {
    pub t: usize,
    pub u: usize,
}

/// Generate the S7 temporal smoothness pair set for one batch row with no
/// explicit sequence-boundary markers.
///
/// `sequence_mask[index] == true` means the token participates in the loss.
/// Invalid endpoints are skipped, and invalid tokens between two endpoints
/// reset the window.
pub fn s7_temporal_smoothness(
    sequence_mask: &[bool],
    smoothness_window: SmoothnessWindow,
) -> Vec<TemporalSmoothnessPair> {
    let no_boundaries = vec![false; sequence_mask.len()];
    s7_temporal_smoothness_with_boundaries(sequence_mask, &no_boundaries, smoothness_window)
        .expect("generated boundary mask length matches sequence mask length")
}

/// Generate the S7 temporal smoothness pair set for one batch row with an
/// explicit boundary-before mask.
///
/// `sequence_boundary_before[t] == true` marks token `t` as the start of a new
/// sequence. Any candidate pair `(t, u)` with a marked boundary in
/// `(u, t]` is excluded, so no pair crosses between two packed sequences.
pub fn s7_temporal_smoothness_with_boundaries(
    sequence_mask: &[bool],
    sequence_boundary_before: &[bool],
    smoothness_window: SmoothnessWindow,
) -> Result<Vec<TemporalSmoothnessPair>, TemporalSmoothnessError> {
    if sequence_boundary_before.len() != sequence_mask.len() {
        return Err(TemporalSmoothnessError::BoundaryMaskLenMismatch {
            sequence_mask_len: sequence_mask.len(),
            boundary_mask_len: sequence_boundary_before.len(),
        });
    }

    let mut pairs = Vec::new();
    for t in 1..sequence_mask.len() {
        if !sequence_mask[t] {
            continue;
        }

        let first_candidate = t.saturating_sub(smoothness_window.as_usize());
        for (u, valid_u) in sequence_mask
            .iter()
            .enumerate()
            .take(t)
            .skip(first_candidate)
        {
            if *valid_u
                && !has_invalid_token_between(sequence_mask, u, t)
                && !has_boundary_between(sequence_boundary_before, u, t)
            {
                pairs.push(TemporalSmoothnessPair { t, u });
            }
        }
    }

    Ok(pairs)
}

fn has_invalid_token_between(sequence_mask: &[bool], u: usize, t: usize) -> bool {
    sequence_mask[u + 1..t].iter().any(|valid| !*valid)
}

fn has_boundary_between(sequence_boundary_before: &[bool], u: usize, t: usize) -> bool {
    sequence_boundary_before[u + 1..=t]
        .iter()
        .any(|boundary| *boundary)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemporalSmoothnessError {
    SmoothnessWindowTooSmall {
        value: u16,
    },
    BoundaryMaskLenMismatch {
        sequence_mask_len: usize,
        boundary_mask_len: usize,
    },
}

impl fmt::Display for TemporalSmoothnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemporalSmoothnessError::SmoothnessWindowTooSmall { value } => {
                write!(
                    f,
                    "smoothness_window must be at least 2 for S7 D10, got {value}"
                )
            }
            TemporalSmoothnessError::BoundaryMaskLenMismatch {
                sequence_mask_len,
                boundary_mask_len,
            } => write!(
                f,
                "sequence boundary mask length {boundary_mask_len} does not match sequence mask length {sequence_mask_len}"
            ),
        }
    }
}

impl Error for TemporalSmoothnessError {}
