//! F-S3 preregistration pin loading and structured logging.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Default workspace-relative F-S3 preregistration pin path.
pub const DEFAULT_PREREGISTRATION_PIN_PATH: &str = "experiments/S3/preregistration.toml";

/// Tracing target for F-S3 preregistration events.
pub const S3_PREREGISTRATION_LOG_TARGET: &str = "gbf_experiments::s3::preregistration";

/// Event emitted when the F-S3 preregistration pin is loaded.
pub const S3_PREREGISTRATION_PIN_LOADED_EVENT: &str = "s3::preregistration_pin_loaded";

/// Parsed F-S3 preregistration pin.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct S3PreregistrationPin {
    /// Pin schema name.
    pub schema: String,
    /// Commit containing the pre-registered F-S3 predictions.
    pub predictions_commit: String,
    /// Hash of the pre-registered predictions section.
    pub predictions_section_hash: String,
    /// S3 pass-version pin.
    #[serde(rename = "pass_version_S3")]
    pub pass_version_s3: String,
    /// RFC revision commit used by this preregistration.
    pub rfc_revision: String,
    /// Empty until S3 result artifacts exist.
    #[serde(default)]
    pub first_result_commit: String,
}

/// Error returned while reading or parsing an F-S3 preregistration pin.
#[derive(Debug)]
pub enum S3PreregistrationError {
    /// The pin file could not be read.
    Read {
        /// Path that was read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The pin file was not valid TOML for the F-S3 pin shape.
    Parse {
        /// Path that was parsed.
        path: PathBuf,
        /// Underlying TOML parse error.
        source: toml::de::Error,
    },
}

impl fmt::Display for S3PreregistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    f,
                    "failed to read S3 preregistration pin {}: {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse S3 preregistration pin {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for S3PreregistrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

/// Load the default F-S3 preregistration pin path relative to the current process.
///
/// Callers that need workspace-root independence should use
/// [`load_preregistration_pin`] with an explicit path.
pub fn load_default_preregistration_pin() -> Result<S3PreregistrationPin, S3PreregistrationError> {
    load_preregistration_pin(DEFAULT_PREREGISTRATION_PIN_PATH)
}

/// Load an F-S3 preregistration pin and emit `s3::preregistration_pin_loaded`.
pub fn load_preregistration_pin(
    path: impl AsRef<Path>,
) -> Result<S3PreregistrationPin, S3PreregistrationError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| S3PreregistrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let pin = toml::from_str::<S3PreregistrationPin>(&text).map_err(|source| {
        S3PreregistrationError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })?;
    emit_preregistration_pin_loaded(&pin);
    Ok(pin)
}

fn emit_preregistration_pin_loaded(pin: &S3PreregistrationPin) {
    tracing::info!(
        target: S3_PREREGISTRATION_LOG_TARGET,
        event_name = S3_PREREGISTRATION_PIN_LOADED_EVENT,
        predictions_commit = pin.predictions_commit.as_str(),
        predictions_section_hash = pin.predictions_section_hash.as_str(),
        pass_version_S3 = pin.pass_version_s3.as_str(),
        rfc_revision = pin.rfc_revision.as_str(),
        "s3 preregistration pin loaded"
    );
}
