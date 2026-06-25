//! S7 matched-bytes pin emitter.

use std::fmt;
use std::path::Path;

use gbf_artifact::{MatchedBytesPin, MatchedBytesPinError};
use gbf_policy::{
    DenseMatchedBytesPolicy, MatchedBytesConfig, MatchedBytesError, solve_d_ff_dense,
};

/// Compute the canonical S7 `matched_bytes.json` pin.
pub fn canonical_s7_matched_bytes_pin() -> Result<MatchedBytesPin, S7MatchedBytesError> {
    let policy = DenseMatchedBytesPolicy::s7_canonical().matched_bytes_policy();
    let solution = solve_d_ff_dense(MatchedBytesConfig::s7_moe_tiny(), policy)?;
    Ok(MatchedBytesPin::from_solution(solution, policy).with_computed_self_hash()?)
}

/// Serialize a matched-bytes pin to canonical JSON bytes with a trailing newline.
pub fn matched_bytes_pin_json_bytes(pin: &MatchedBytesPin) -> Result<Vec<u8>, S7MatchedBytesError> {
    let mut bytes = pin.canonical_json_bytes()?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Compute and serialize the canonical S7 `matched_bytes.json` pin.
pub fn canonical_s7_matched_bytes_json_bytes() -> Result<Vec<u8>, S7MatchedBytesError> {
    matched_bytes_pin_json_bytes(&canonical_s7_matched_bytes_pin()?)
}

/// Write the canonical S7 `matched_bytes.json` pin to `path`.
pub fn write_canonical_s7_matched_bytes_pin(
    path: &Path,
) -> Result<MatchedBytesPin, S7MatchedBytesError> {
    let pin = canonical_s7_matched_bytes_pin()?;
    let bytes = matched_bytes_pin_json_bytes(&pin)?;
    std::fs::write(path, bytes).map_err(S7MatchedBytesError::Io)?;
    Ok(pin)
}

/// Errors from S7 matched-bytes pin emission.
#[derive(Debug)]
pub enum S7MatchedBytesError {
    /// D6 solver failed.
    Solver(MatchedBytesError),
    /// MatchedBytesPin schema or self-hash validation failed.
    Pin(MatchedBytesPinError),
    /// Filesystem write failed.
    Io(std::io::Error),
}

impl fmt::Display for S7MatchedBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solver(error) => write!(f, "{error}"),
            Self::Pin(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S7MatchedBytesError {}

impl From<MatchedBytesError> for S7MatchedBytesError {
    fn from(error: MatchedBytesError) -> Self {
        Self::Solver(error)
    }
}

impl From<MatchedBytesPinError> for S7MatchedBytesError {
    fn from(error: MatchedBytesPinError) -> Self {
        Self::Pin(error)
    }
}
