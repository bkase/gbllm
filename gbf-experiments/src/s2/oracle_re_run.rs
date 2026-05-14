//! S2 measurement-oracle re-run artifact helpers.

use std::fmt;
use std::fs;
use std::path::Path;

use gbf_foundation::sha256;

use crate::S2_LOG_TARGET;
use crate::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};
use crate::s1::oracle::{
    MetricOracleResults, OracleEmitError, emit_oracle_report, run_metric_oracles,
};
use crate::s1::schema::S1CanonicalJson;
use crate::s2::schema::{S2OracleReRunReport, S2ReportWriteError};

/// S1 D7 oracle suite version re-run for S2 O3.
pub const S1_ORACLE_SUITE_VERSION: &str = "s1-d7.v1";

/// Canonical S1 D7 oracle case ids that must appear in `s2_oracle_re_run.v1`.
pub const ORACLE_CASE_IDS: [&str; 5] = [
    "O-metric-0",
    "O-metric-1",
    "O-metric-2",
    "O-metric-3",
    "O-metric-4",
];

/// Run the inherited S1 measurement oracle suite under the S2 binary.
pub fn run_s1_oracle_re_run_under_s2_binary() -> Result<S2OracleReRunReport, S2OracleReRunError> {
    let val_bytes = b"s2 oracle re-run validation fixture";
    let expected_shuffle_pin = sha256(fisher_yates(val_bytes, NEGATIVE_TEST_SHUFFLE_SEED));
    let s1_report = run_metric_oracles(0, val_bytes, expected_shuffle_pin)?;
    let results = MetricOracleResults {
        o_metric_0: s1_report.o_metric_0,
        o_metric_1: s1_report.o_metric_1,
        o_metric_2: s1_report.o_metric_2,
        o_metric_3: s1_report.o_metric_3,
        o_metric_4: s1_report.o_metric_4,
    };
    emit_s2_oracle_re_run_from_results(results)
}

/// Emit an S2 oracle re-run report from explicit oracle results.
///
/// This keeps failure-path tests deterministic while still reusing the S1 D7
/// oracle result vocabulary and `s1_oracle.v1` aggregate validation.
pub fn emit_s2_oracle_re_run_from_results(
    results: MetricOracleResults,
) -> Result<S2OracleReRunReport, S2OracleReRunError> {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "oracle_re_run_start",
        event = "oracle_re_run_start",
        s1_oracle_suite_version = S1_ORACLE_SUITE_VERSION,
        case_count = ORACLE_CASE_IDS.len() as u32,
        "s2 oracle re-run start"
    );
    let s1_report = emit_oracle_report(0, results)?;
    for (case, passed) in per_case_results(results) {
        let observed = if passed {
            "null".to_owned()
        } else {
            format!(r#"{{"case":"{case}","passed":false}}"#)
        };
        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "oracle_case_invoked",
            event = "oracle_case_invoked",
            case,
            passed,
            observed = observed.as_str(),
            "s2 oracle case invoked"
        );
        if !passed {
            let expected = format!(r#"{{"case":"{case}","passed":true}}"#);
            tracing::error!(
                target: S2_LOG_TARGET,
                event_name = "oracle_case_failed",
                event = "oracle_case_failed",
                case,
                expected = expected.as_str(),
                observed = observed.as_str(),
                "s2 oracle case failed"
            );
        }
    }
    let report = S2OracleReRunReport::new(
        S1_ORACLE_SUITE_VERSION,
        s1_report.metric_oracle_passed,
        ORACLE_CASE_IDS
            .iter()
            .map(|case| (*case).to_owned())
            .collect(),
    )?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "oracle_re_run_finalized",
        event = "oracle_re_run_finalized",
        metric_oracle_passed = report.metric_oracle_passed,
        cases_total = report.oracle_cases.len() as u32,
        oracle_re_run_self_hash = %report.oracle_re_run_self_hash,
        "s2 oracle re-run finalized"
    );
    Ok(report)
}

/// Write `s2_oracle_re_run.v1` as canonical JSON.
pub fn write_oracle_re_run_report(
    path: impl AsRef<Path>,
    report: &S2OracleReRunReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    report.validate()?;
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "oracle_re_run_persisted",
        event = "oracle_re_run_persisted",
        metric_oracle_passed = report.metric_oracle_passed,
        cases_total = report.oracle_cases.len() as u32,
        oracle_re_run_self_hash = %report.oracle_re_run_self_hash,
        "s2 oracle re-run persisted"
    );
    Ok(())
}

/// Errors from S2 oracle re-run emission.
#[derive(Debug)]
pub enum S2OracleReRunError {
    /// S1 oracle suite emission failed.
    Oracle(OracleEmitError),
    /// S2 oracle re-run report hashing failed.
    Schema(crate::s1::schema::S1SchemaError),
}

impl fmt::Display for S2OracleReRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oracle(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S2OracleReRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Oracle(error) => Some(error),
            Self::Schema(error) => Some(error),
        }
    }
}

impl From<OracleEmitError> for S2OracleReRunError {
    fn from(error: OracleEmitError) -> Self {
        Self::Oracle(error)
    }
}

impl From<crate::s1::schema::S1SchemaError> for S2OracleReRunError {
    fn from(error: crate::s1::schema::S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

fn per_case_results(results: MetricOracleResults) -> [(&'static str, bool); 5] {
    [
        ("O-metric-0", results.o_metric_0),
        ("O-metric-1", results.o_metric_1),
        ("O-metric-2", results.o_metric_2),
        ("O-metric-3", results.o_metric_3),
        ("O-metric-4", results.o_metric_4),
    ]
}
