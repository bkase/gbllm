//! S3 inherited oracle re-run helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, self_hash_omitting_fields, sha256,
};
use serde::{Deserialize, Serialize};

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
use crate::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
use crate::s1::oracle::{OracleEmitError, run_metric_oracles};
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
use crate::s1::schema::OracleReport;
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
use crate::s2::oracle_re_run::{ORACLE_CASE_IDS as S2_ORACLE_CASE_IDS, S2OracleReRunError};

pub mod tolerances;

use tolerances::tolerance_for;

/// Canonical schema id for the S3 inherited oracle re-run report.
pub const S3_ORACLE_RE_RUN_SCHEMA: &str = "s3_oracle_re_run.v1";

/// Tracing target for S3 oracle re-run events.
pub const S3_ORACLE_RE_RUN_LOG_TARGET: &str = "gbf_experiments::s3::oracle_re_run";

/// Pass-version string emitted by the S3 oracle re-run command.
pub const PASS_VERSION_S3: &str = env!("CARGO_PKG_VERSION");

/// S1 D7 oracle ids inherited by the S3 oracle re-run surface.
pub const S1_D7_METRIC_IDS: [&str; 5] = [
    "O-metric-0",
    "O-metric-1",
    "O-metric-2",
    "O-metric-3",
    "O-metric-4",
];

/// Version tag for the inherited S1 D7 oracle suite.
pub const S1_ORACLE_SUITE_VERSION: &str = "s1-d7.v1";

/// Version tag for the inherited F-S2 O3 oracle re-run surface.
pub const S2_ORACLE_SUITE_VERSION: &str = "s2-o3.v1";

const TRUE_INDICATOR: f64 = 1.0;
const FALSE_INDICATOR: f64 = 0.0;
const S3_ORACLE_RE_RUN_SEED: u64 = 0;

/// Stable metric identifier for S3 oracle re-run rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MetricId(String);

impl MetricId {
    /// Construct a non-empty metric id.
    pub fn new(value: impl Into<String>) -> Result<Self, OracleReRunError> {
        let value = value.into();
        if value.is_empty() {
            Err(OracleReRunError::InvalidMetricId(value))
        } else {
            Ok(Self(value))
        }
    }

    /// Borrow the metric id as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for MetricId {
    fn from(value: &'static str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Display for MetricId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Per-metric S3 re-run comparison row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricReRun {
    /// Inherited F-S1 D7 baseline value.
    pub s1_baseline: f64,
    /// Inherited F-S2 O3 baseline value.
    ///
    /// S2 exposes an aggregate oracle-re-run pass bit plus inherited case ids,
    /// not per-metric values, so this mirrors the pass-indicator baseline.
    pub s2_baseline: f64,
    /// S3-observed value under the active binary.
    pub s3_observed: f64,
    /// Difference between S3 observed and F-S1 baseline.
    pub delta_vs_s1: f64,
    /// Difference between S3 observed and F-S2 baseline.
    pub delta_vs_s2: f64,
    /// Inclusive absolute tolerance applied to both inherited baselines.
    pub tolerance: f64,
    /// Whether both inherited-baseline deltas are within tolerance.
    pub passed: bool,
}

impl MetricReRun {
    /// Construct a row and recompute deltas and pass status.
    #[must_use]
    pub fn new(s1_baseline: f64, s2_baseline: f64, s3_observed: f64, tolerance: f64) -> Self {
        let delta_vs_s1 = s3_observed - s1_baseline;
        let delta_vs_s2 = s3_observed - s2_baseline;
        let passed = delta_vs_s1.abs() <= tolerance && delta_vs_s2.abs() <= tolerance;
        Self {
            s1_baseline,
            s2_baseline,
            s3_observed,
            delta_vs_s1,
            delta_vs_s2,
            tolerance,
            passed,
        }
    }

    /// Returns whether the S1 delta is within this row's tolerance.
    #[must_use]
    pub fn passed_vs_s1(&self) -> bool {
        self.delta_vs_s1.abs() <= self.tolerance
    }

    /// Returns whether the S2 delta is within this row's tolerance.
    #[must_use]
    pub fn passed_vs_s2(&self) -> bool {
        self.delta_vs_s2.abs() <= self.tolerance
    }

    fn validate(&self, metric_id: &MetricId) -> Result<(), OracleReRunError> {
        for (field, value) in [
            ("s1_baseline", self.s1_baseline),
            ("s2_baseline", self.s2_baseline),
            ("s3_observed", self.s3_observed),
            ("delta_vs_s1", self.delta_vs_s1),
            ("delta_vs_s2", self.delta_vs_s2),
            ("tolerance", self.tolerance),
        ] {
            if !value.is_finite() {
                return Err(OracleReRunError::NonFiniteMetric {
                    metric_id: metric_id.to_string(),
                    field,
                });
            }
        }
        if self.tolerance < 0.0 {
            return Err(OracleReRunError::NegativeTolerance {
                metric_id: metric_id.to_string(),
                tolerance: self.tolerance,
            });
        }

        let expected_delta_vs_s1 = self.s3_observed - self.s1_baseline;
        if !same_f64(self.delta_vs_s1, expected_delta_vs_s1) {
            return Err(OracleReRunError::DeltaMismatch {
                metric_id: metric_id.to_string(),
                field: "delta_vs_s1",
                expected: expected_delta_vs_s1,
                observed: self.delta_vs_s1,
            });
        }
        let expected_delta_vs_s2 = self.s3_observed - self.s2_baseline;
        if !same_f64(self.delta_vs_s2, expected_delta_vs_s2) {
            return Err(OracleReRunError::DeltaMismatch {
                metric_id: metric_id.to_string(),
                field: "delta_vs_s2",
                expected: expected_delta_vs_s2,
                observed: self.delta_vs_s2,
            });
        }
        let expected_passed = self.passed_vs_s1() && self.passed_vs_s2();
        if self.passed != expected_passed {
            return Err(OracleReRunError::MetricPassedMismatch {
                metric_id: metric_id.to_string(),
                expected: expected_passed,
                observed: self.passed,
            });
        }
        Ok(())
    }
}

/// Canonical S3 inherited oracle re-run report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleReRunReport {
    /// Schema id. Expected value: `s3_oracle_re_run.v1`.
    pub schema: String,
    /// Whether the inherited S1 D7 suite passed under S3.
    pub s1_oracle_re_run_passed: bool,
    /// Whether the inherited F-S2 O3 suite passed under S3.
    pub s2_oracle_re_run_passed: bool,
    /// Per-metric comparison rows keyed by inherited metric id.
    pub per_metric: BTreeMap<MetricId, MetricReRun>,
    /// Self-hash over this report with this field omitted.
    pub oracle_re_run_self_hash: Hash256,
}

impl OracleReRunReport {
    /// DomainHash context for `s3_oracle_re_run.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "OracleReRunReport",
            S3_ORACLE_RE_RUN_SCHEMA,
            "1",
        )
    }

    /// Construct a report, deriving aggregate pass booleans from per-metric deltas.
    pub fn new(per_metric: BTreeMap<MetricId, MetricReRun>) -> Result<Self, OracleReRunError> {
        Self::from_parts(per_metric, true, true)
    }

    /// Construct a report from per-metric deltas and inherited-suite verdicts.
    pub fn from_parts(
        per_metric: BTreeMap<MetricId, MetricReRun>,
        s1_suite_passed: bool,
        s2_suite_passed: bool,
    ) -> Result<Self, OracleReRunError> {
        let (s1_metrics_passed, s2_metrics_passed) = aggregate_metric_passes(&per_metric);
        Self {
            schema: S3_ORACLE_RE_RUN_SCHEMA.to_owned(),
            s1_oracle_re_run_passed: s1_suite_passed && s1_metrics_passed,
            s2_oracle_re_run_passed: s2_suite_passed && s2_metrics_passed,
            per_metric,
            oracle_re_run_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate schema, invariants, and stored self-hash.
    pub fn validate(&self) -> Result<(), OracleReRunError> {
        self.validate_payload()?;
        let expected = self.computed_self_hash()?;
        if self.oracle_re_run_self_hash != expected {
            return Err(OracleReRunError::SelfHashMismatch {
                expected,
                observed: self.oracle_re_run_self_hash,
            });
        }
        Ok(())
    }

    /// Validate the closure gates: both inherited suites must pass under S3.
    pub fn validate_closure(&self) -> Result<(), OracleReRunError> {
        self.validate()?;
        if !self.s1_oracle_re_run_passed {
            return Err(OracleReRunError::InheritedSuiteFailed {
                suite: S1_ORACLE_SUITE_VERSION,
            });
        }
        if !self.s2_oracle_re_run_passed {
            return Err(OracleReRunError::InheritedSuiteFailed {
                suite: S2_ORACLE_SUITE_VERSION,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes used for this artifact's self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, OracleReRunError> {
        self.validate_payload()?;
        gbf_foundation::canonical_json_bytes_omitting_fields(self, &["oracle_re_run_self_hash"])
            .map_err(OracleReRunError::CanonicalJson)
    }

    /// Compute this report's self-hash from its canonical payload.
    pub fn computed_self_hash(&self) -> Result<Hash256, OracleReRunError> {
        self.validate_payload()?;
        self_hash_omitting_fields(Self::domain(), self, "oracle_re_run_self_hash", &[])
            .map_err(OracleReRunError::CanonicalJson)
    }

    /// Return a copy with its stored self-hash replaced by the computed one.
    pub fn with_computed_self_hash(mut self) -> Result<Self, OracleReRunError> {
        self.oracle_re_run_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    fn validate_payload(&self) -> Result<(), OracleReRunError> {
        if self.schema != S3_ORACLE_RE_RUN_SCHEMA {
            return Err(OracleReRunError::InvalidSchema(self.schema.clone()));
        }
        if self.per_metric.is_empty() {
            return Err(OracleReRunError::EmptyPerMetric);
        }
        let expected_metric_ids = S1_D7_METRIC_IDS
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        let observed_metric_ids = self
            .per_metric
            .keys()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        if observed_metric_ids != expected_metric_ids {
            return Err(OracleReRunError::MetricSetMismatch {
                expected: expected_metric_ids.into_iter().collect(),
                observed: observed_metric_ids.into_iter().collect(),
            });
        }
        for (metric_id, row) in &self.per_metric {
            if metric_id.as_str().is_empty() {
                return Err(OracleReRunError::InvalidMetricId(metric_id.to_string()));
            }
            row.validate(metric_id)?;
        }
        let (s1_metrics_passed, s2_metrics_passed) = aggregate_metric_passes(&self.per_metric);
        if self.s1_oracle_re_run_passed && !s1_metrics_passed {
            return Err(OracleReRunError::AggregatePassedMismatch {
                field: "s1_oracle_re_run_passed",
            });
        }
        if self.s2_oracle_re_run_passed && !s2_metrics_passed {
            return Err(OracleReRunError::AggregatePassedMismatch {
                field: "s2_oracle_re_run_passed",
            });
        }
        Ok(())
    }
}

/// Run the inherited S1 D7 and F-S2 O3 oracle suites under the active S3 binary.
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
pub fn s3_oracle_re_run() -> Result<OracleReRunReport, OracleReRunError> {
    tracing::info!(
        target: S3_ORACLE_RE_RUN_LOG_TARGET,
        event_name = "s3::oracle_re_run::run_started",
        binary_pass_version = PASS_VERSION_S3,
        s1_oracle_count = S1_D7_METRIC_IDS.len() as u64,
        s2_oracle_count = S2_ORACLE_CASE_IDS.len() as u64,
        "s3::oracle_re_run::run_started"
    );

    let s1_report = run_s1_suite_under_s3_binary()?;
    let s2_report = crate::s2::oracle_re_run::run_s1_oracle_re_run_under_s2_binary()?;
    let s1_results = s1_oracle_results_from_report(&s1_report);
    let mut per_metric = BTreeMap::new();

    for (metric_id, observed_passed) in s1_results {
        let metric_id = MetricId::from(metric_id);
        let tolerance = tolerance_for(metric_id.as_str())?;
        // S1 still provides the per-metric boolean rows. The inherited S2
        // re-run surface only exposes an aggregate pass plus canonical case ids,
        // so the per-metric `s2_baseline` is the same pass indicator used by
        // the S1 baseline until a richer S2 report exists.
        let row = MetricReRun::new(
            TRUE_INDICATOR,
            TRUE_INDICATOR,
            indicator(observed_passed),
            tolerance,
        );
        tracing::trace!(
            target: S3_ORACLE_RE_RUN_LOG_TARGET,
            event_name = "s3::oracle_re_run::metric_evaluated",
            metric_id = metric_id.as_str(),
            s1_baseline = row.s1_baseline,
            s2_baseline = row.s2_baseline,
            s3_observed = row.s3_observed,
            delta_vs_s1 = row.delta_vs_s1,
            delta_vs_s2 = row.delta_vs_s2,
            tolerance = row.tolerance,
            passed = row.passed,
            "s3::oracle_re_run::metric_evaluated"
        );
        per_metric.insert(metric_id, row);
    }

    let report = OracleReRunReport::from_parts(
        per_metric,
        s1_report.metric_oracle_passed,
        s2_report.metric_oracle_passed,
    )?;
    tracing::info!(
        target: S3_ORACLE_RE_RUN_LOG_TARGET,
        event_name = "s3::oracle_re_run::run_complete",
        s1_oracle_re_run_passed = report.s1_oracle_re_run_passed,
        s2_oracle_re_run_passed = report.s2_oracle_re_run_passed,
        oracle_re_run_self_hash = %report.oracle_re_run_self_hash,
        "s3::oracle_re_run::run_complete"
    );
    Ok(report)
}

/// Return an explicit error when inherited S1/S2 modules are not compiled.
#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
)))]
pub fn s3_oracle_re_run() -> Result<OracleReRunReport, OracleReRunError> {
    Err(OracleReRunError::InheritedOracleModulesUnavailable)
}

/// Persist an S3 oracle re-run report as canonical JSON.
pub fn write_oracle_re_run_report(
    path: &Path,
    report: &OracleReRunReport,
) -> Result<(), OracleReRunError> {
    report.validate()?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| OracleReRunError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let bytes = CanonicalJson::to_vec(report)?;
    std::fs::write(path, bytes).map_err(|source| OracleReRunError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Errors from S3 inherited oracle re-run construction and persistence.
#[derive(Debug)]
pub enum OracleReRunError {
    /// Inherited S1/S2 oracle modules were not compiled into this binary.
    InheritedOracleModulesUnavailable,
    /// S1 D7 oracle execution failed.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "s3-phase-d",
        feature = "falsify"
    ))]
    S1Oracle(OracleEmitError),
    /// S2 O3 oracle re-run execution failed.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "s3-phase-d",
        feature = "falsify"
    ))]
    S2Oracle(S2OracleReRunError),
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// Canonical JSON or DomainHash construction failed.
    CanonicalJson(CanonicalJsonError),
    /// Report schema id was not `s3_oracle_re_run.v1`.
    InvalidSchema(String),
    /// Per-metric map must not be empty.
    EmptyPerMetric,
    /// Metric id was invalid.
    InvalidMetricId(String),
    /// Per-metric keys did not match the inherited D7 metric set.
    MetricSetMismatch {
        /// Expected metric ids.
        expected: Vec<String>,
        /// Observed metric ids.
        observed: Vec<String>,
    },
    /// Metric id has no pinned tolerance.
    UnknownToleranceMetric(String),
    /// Metric field was non-finite.
    NonFiniteMetric {
        /// Metric id whose row failed validation.
        metric_id: String,
        /// Field that contained a non-finite float.
        field: &'static str,
    },
    /// Metric tolerance was negative.
    NegativeTolerance {
        /// Metric id whose tolerance failed validation.
        metric_id: String,
        /// Observed tolerance.
        tolerance: f64,
    },
    /// Stored delta did not match observed minus baseline.
    DeltaMismatch {
        /// Metric id whose row failed validation.
        metric_id: String,
        /// Delta field name.
        field: &'static str,
        /// Recomputed delta.
        expected: f64,
        /// Stored delta.
        observed: f64,
    },
    /// Per-metric `passed` field disagreed with tolerance checks.
    MetricPassedMismatch {
        /// Metric id whose row failed validation.
        metric_id: String,
        /// Recomputed pass value.
        expected: bool,
        /// Stored pass value.
        observed: bool,
    },
    /// Aggregate pass field claimed success while per-metric deltas failed.
    AggregatePassedMismatch {
        /// Aggregate field that failed validation.
        field: &'static str,
    },
    /// Stored self-hash differed from the computed DomainHash.
    SelfHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// Closure validation found a failed inherited suite.
    InheritedSuiteFailed {
        /// Inherited suite version that failed.
        suite: &'static str,
    },
}

impl fmt::Display for OracleReRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InheritedOracleModulesUnavailable => f.write_str(
                "S3 oracle re-run requires inherited S1/S2 oracle modules in the active binary",
            ),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::S1Oracle(error) => write!(f, "{error}"),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::S2Oracle(error) => write!(f, "{error}"),
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::InvalidSchema(schema) => {
                write!(f, "invalid S3 oracle re-run schema {schema:?}")
            }
            Self::EmptyPerMetric => f.write_str("S3 oracle re-run per_metric map is empty"),
            Self::InvalidMetricId(metric_id) => {
                write!(f, "invalid S3 oracle re-run metric id {metric_id:?}")
            }
            Self::MetricSetMismatch { expected, observed } => write!(
                f,
                "S3 oracle re-run metric set mismatch: expected {expected:?}, observed {observed:?}"
            ),
            Self::UnknownToleranceMetric(metric_id) => {
                write!(f, "missing S3 oracle re-run tolerance for {metric_id:?}")
            }
            Self::NonFiniteMetric { metric_id, field } => {
                write!(f, "{metric_id}.{field} must be finite")
            }
            Self::NegativeTolerance {
                metric_id,
                tolerance,
            } => write!(
                f,
                "{metric_id}.tolerance must be non-negative, got {tolerance}"
            ),
            Self::DeltaMismatch {
                metric_id,
                field,
                expected,
                observed,
            } => write!(
                f,
                "{metric_id}.{field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::MetricPassedMismatch {
                metric_id,
                expected,
                observed,
            } => write!(
                f,
                "{metric_id}.passed mismatch: expected {expected}, observed {observed}"
            ),
            Self::AggregatePassedMismatch { field } => {
                write!(
                    f,
                    "{field} cannot be true when inherited metric deltas fail"
                )
            }
            Self::SelfHashMismatch { expected, observed } => {
                write!(
                    f,
                    "S3 oracle re-run self hash mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::InheritedSuiteFailed { suite } => {
                write!(f, "inherited oracle suite {suite} failed under S3")
            }
        }
    }
}

impl std::error::Error for OracleReRunError {}

impl From<CanonicalJsonError> for OracleReRunError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
impl From<OracleEmitError> for OracleReRunError {
    fn from(error: OracleEmitError) -> Self {
        Self::S1Oracle(error)
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
impl From<S2OracleReRunError> for OracleReRunError {
    fn from(error: S2OracleReRunError) -> Self {
        Self::S2Oracle(error)
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn run_s1_suite_under_s3_binary() -> Result<OracleReport, OracleReRunError> {
    // The S3 inherited re-run uses a distinct validation fixture from the S2
    // re-run helper. This keeps the two inherited-suite checks independently
    // identifiable while still comparing their pass-indicator baselines.
    let val_bytes = b"s3 inherited oracle re-run validation fixture";
    let expected_shuffle_pin = sha256(fisher_yates(val_bytes, NEGATIVE_TEST_SHUFFLE_SEED));
    run_metric_oracles(S3_ORACLE_RE_RUN_SEED, val_bytes, expected_shuffle_pin)
        .map_err(OracleReRunError::S1Oracle)
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn s1_oracle_results_from_report(report: &OracleReport) -> [(&'static str, bool); 5] {
    [
        ("O-metric-0", report.o_metric_0),
        ("O-metric-1", report.o_metric_1),
        ("O-metric-2", report.o_metric_2),
        ("O-metric-3", report.o_metric_3),
        ("O-metric-4", report.o_metric_4),
    ]
}

fn aggregate_metric_passes(per_metric: &BTreeMap<MetricId, MetricReRun>) -> (bool, bool) {
    let s1_passed = per_metric.values().all(MetricReRun::passed_vs_s1);
    let s2_passed = per_metric.values().all(MetricReRun::passed_vs_s2);
    (s1_passed, s2_passed)
}

fn same_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn indicator(passed: bool) -> f64 {
    if passed {
        TRUE_INDICATOR
    } else {
        FALSE_INDICATOR
    }
}
