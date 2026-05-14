//! O11 public API non-drift checks for S2 closure.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;

use gbf_foundation::{Hash256, sha256};

use crate::S2_LOG_TARGET;

const QAT_SNAPSHOT_FILE: &str = "s1_qat_public_api.txt";
const LINEARSTATE_SNAPSHOT_FILE: &str = "s1_linearstate_public_api.txt";
const QAT_MODULE: &str = "gbf-model::qat";
const LINEARSTATE_MODULE: &str = "gbf-model::sequence::LinearStateBlock";

/// Empty O11 v1 API drift allow-list.
pub const S2_ALLOWED_API_DRIFT_V1: &[&str] = &[];

/// Current public symbols for the two O11 API surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiDriftSymbols {
    /// Current public symbols under `gbf_model::qat`.
    pub qat: Vec<String>,
    /// Current public symbols for `gbf_model::sequence::LinearStateBlock`.
    pub linearstate: Vec<String>,
}

/// Public API drift result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiDriftCheckResult {
    /// Whether all observed drift is allow-listed.
    pub passed: bool,
    /// Number of drifted symbols.
    pub drift_count: u32,
    /// Hash of `s1_qat_public_api.txt`.
    pub qat_public_api_snapshot_hash: Hash256,
    /// Hash of `s1_linearstate_public_api.txt`.
    pub linearstate_public_api_snapshot_hash: Hash256,
    /// Added/removed symbols.
    pub drifts: Vec<ApiSymbolDrift>,
}

/// One public-symbol drift diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiSymbolDrift {
    /// API module surface.
    pub module: String,
    /// Public symbol name.
    pub symbol: String,
    /// Drift kind.
    pub kind: ApiSymbolDriftKind,
    /// Whether the drift key is allow-listed.
    pub in_allow_list: bool,
}

/// Public-symbol drift kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiSymbolDriftKind {
    /// Current surface contains a symbol absent from the S1 snapshot.
    Added,
    /// S1 snapshot contains a symbol absent from the current surface.
    Removed,
}

/// Errors from O11 API drift checking.
#[derive(Debug)]
pub enum ApiDriftError {
    /// Filesystem read failed.
    Io(std::io::Error),
}

impl fmt::Display for ApiDriftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ApiDriftError {}

impl From<std::io::Error> for ApiDriftError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Check current symbols against pinned S1 API snapshots.
pub fn check_api_drift(
    snapshots_dir: impl AsRef<Path>,
    current: ApiDriftSymbols,
) -> Result<ApiDriftCheckResult, ApiDriftError> {
    check_api_drift_with_allow_list(snapshots_dir, current, S2_ALLOWED_API_DRIFT_V1)
}

/// Check current symbols against pinned S1 API snapshots with an explicit allow-list.
pub fn check_api_drift_with_allow_list(
    snapshots_dir: impl AsRef<Path>,
    current: ApiDriftSymbols,
    allow_list: &[&str],
) -> Result<ApiDriftCheckResult, ApiDriftError> {
    let snapshots_dir = snapshots_dir.as_ref();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "api_drift_check_start",
        event = "api_drift_check_start",
        snapshots_dir = %snapshots_dir.display(),
        "s2 api drift check start"
    );
    let qat_bytes = fs::read(snapshots_dir.join(QAT_SNAPSHOT_FILE))?;
    let linearstate_bytes = fs::read(snapshots_dir.join(LINEARSTATE_SNAPSHOT_FILE))?;
    let qat_snapshot = normalize_symbols(std::str::from_utf8(&qat_bytes).unwrap_or_default());
    let linearstate_snapshot =
        normalize_symbols(std::str::from_utf8(&linearstate_bytes).unwrap_or_default());
    let qat_current = current.qat.into_iter().collect::<Vec<_>>().join("\n");
    let linearstate_current = current
        .linearstate
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n");

    let mut drifts = Vec::new();
    collect_drifts(
        QAT_MODULE,
        &qat_snapshot,
        &normalize_symbols(&qat_current),
        allow_list,
        &mut drifts,
    );
    collect_drifts(
        LINEARSTATE_MODULE,
        &linearstate_snapshot,
        &normalize_symbols(&linearstate_current),
        allow_list,
        &mut drifts,
    );
    let passed = drifts.iter().all(|drift| drift.in_allow_list);
    if !passed {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "api_drift_violation",
            event = "api_drift_violation",
            drifted_symbols = ?drifts.iter().map(ApiSymbolDrift::key).collect::<Vec<_>>(),
            "s2 api drift violation"
        );
    }
    let result = ApiDriftCheckResult {
        passed,
        drift_count: drifts.len() as u32,
        qat_public_api_snapshot_hash: sha256(&qat_bytes),
        linearstate_public_api_snapshot_hash: sha256(&linearstate_bytes),
        drifts,
    };
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "api_drift_check_done",
        event = "api_drift_check_done",
        passed = result.passed,
        drift_count = result.drift_count,
        qat_snapshot_hash = %result.qat_public_api_snapshot_hash,
        linearstate_snapshot_hash = %result.linearstate_public_api_snapshot_hash,
        "s2 api drift check done"
    );
    Ok(result)
}

/// Read a snapshot file into sorted public symbols for tests and wrappers.
pub fn read_snapshot_symbols(path: impl AsRef<Path>) -> Result<Vec<String>, ApiDriftError> {
    let bytes = fs::read(path)?;
    Ok(
        normalize_symbols(std::str::from_utf8(&bytes).unwrap_or_default())
            .into_iter()
            .collect(),
    )
}

fn collect_drifts(
    module: &str,
    snapshot: &BTreeSet<String>,
    current: &BTreeSet<String>,
    allow_list: &[&str],
    drifts: &mut Vec<ApiSymbolDrift>,
) {
    for symbol in current.difference(snapshot) {
        push_drift(
            module,
            symbol,
            ApiSymbolDriftKind::Added,
            allow_list,
            drifts,
        );
    }
    for symbol in snapshot.difference(current) {
        push_drift(
            module,
            symbol,
            ApiSymbolDriftKind::Removed,
            allow_list,
            drifts,
        );
    }
}

fn push_drift(
    module: &str,
    symbol: &str,
    kind: ApiSymbolDriftKind,
    allow_list: &[&str],
    drifts: &mut Vec<ApiSymbolDrift>,
) {
    let drift = ApiSymbolDrift {
        module: module.to_owned(),
        symbol: symbol.to_owned(),
        kind,
        in_allow_list: {
            let qualified = format!("{module}::{symbol}");
            allow_list
                .iter()
                .any(|allowed| *allowed == symbol || *allowed == qualified)
        },
    };
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "api_symbol_drift",
        event = "api_symbol_drift",
        module = drift.module.as_str(),
        symbol = drift.symbol.as_str(),
        kind = drift.kind.as_str(),
        in_allow_list = drift.in_allow_list,
        "s2 api symbol drift"
    );
    drifts.push(drift);
}

fn normalize_symbols(contents: &str) -> BTreeSet<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

impl ApiSymbolDrift {
    fn key(&self) -> String {
        format!("{}::{}", self.module, self.symbol)
    }
}

impl ApiSymbolDriftKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }
}
