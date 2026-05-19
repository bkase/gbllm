//! Pinned tolerance bands for inherited S3 oracle re-runs.

use super::{OracleReRunError, S1_D7_METRIC_IDS};

/// Per-metric tolerance row inherited from F-S1 D7 and F-S2 O3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricTolerance {
    /// Inherited metric id.
    pub metric_id: &'static str,
    /// Inclusive absolute tolerance for the S1 baseline comparison.
    pub tolerance: f64,
}

/// Exact tolerance for boolean pass-indicator fallback metrics.
pub const EXACT_PASS_INDICATOR_TOLERANCE: f64 = 0.0;

/// Pinned per-metric S3 inherited oracle re-run tolerances.
pub const ORACLE_RE_RUN_TOLERANCES: [MetricTolerance; 5] = [
    MetricTolerance {
        metric_id: S1_D7_METRIC_IDS[0],
        tolerance: EXACT_PASS_INDICATOR_TOLERANCE,
    },
    MetricTolerance {
        metric_id: S1_D7_METRIC_IDS[1],
        tolerance: EXACT_PASS_INDICATOR_TOLERANCE,
    },
    MetricTolerance {
        metric_id: S1_D7_METRIC_IDS[2],
        tolerance: EXACT_PASS_INDICATOR_TOLERANCE,
    },
    MetricTolerance {
        metric_id: S1_D7_METRIC_IDS[3],
        tolerance: EXACT_PASS_INDICATOR_TOLERANCE,
    },
    MetricTolerance {
        metric_id: S1_D7_METRIC_IDS[4],
        tolerance: EXACT_PASS_INDICATOR_TOLERANCE,
    },
];

/// Return the pinned tolerance for a metric id.
pub fn tolerance_for(metric_id: &str) -> Result<f64, OracleReRunError> {
    ORACLE_RE_RUN_TOLERANCES
        .iter()
        .find(|row| row.metric_id == metric_id)
        .map(|row| row.tolerance)
        .ok_or_else(|| OracleReRunError::UnknownToleranceMetric(metric_id.to_owned()))
}
