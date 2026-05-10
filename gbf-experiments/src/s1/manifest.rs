//! Manifest loading and validation delegates for S1.

use std::error::Error;
use std::fmt;
use std::path::Path;

pub use gbf_data::{CorpusManifestError, TinyStoriesManifest};
use gbf_foundation::{Hash256, sha256};

use crate::s1::device_profile::{
    DeviceProfileEnforceError, DeviceProfileEnforcement, S1CpuDeterministic, enforce,
};
use crate::s1::logging::{
    LoggingEventError, ManifestShufflePinComputeEvent, ManifestShufflePinVerifyFailEvent,
    ManifestShufflePinVerifyOkEvent, S1LogEmitter,
};
use crate::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};

/// Owned raw byte sequence for S1 corpus consumers.
pub type ByteSeq = Vec<u8>;

/// Pass-version label that owns the currently pinned TinyStories validation shuffle hash.
pub const TINYSTORIES_VAL_SHUFFLE_PIN_PASS_VERSION: &str = "F-S1.09.bd-2who.2026-05-09";

/// Train/validation corpus bytes verified against the TinyStories manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCorpus {
    /// Raw train split bytes verified against the manifest sha256.
    pub train: ByteSeq,
    /// Raw validation split bytes verified against the manifest sha256.
    pub val: ByteSeq,
}

/// Read and schema-check the TinyStories manifest through the gbf-data loader.
pub fn read_tinystories_manifest(
    path: impl AsRef<Path>,
) -> Result<TinyStoriesManifest, CorpusManifestError> {
    gbf_data::read_tinystories_manifest(path)
}

/// Load the raw train split and verify byte length plus sha256.
pub fn load_train_bytes(manifest: &TinyStoriesManifest) -> Result<ByteSeq, CorpusManifestError> {
    gbf_data::load_train_bytes(manifest)
}

/// Load the raw validation split and verify byte length plus sha256.
pub fn load_val_bytes(manifest: &TinyStoriesManifest) -> Result<ByteSeq, CorpusManifestError> {
    gbf_data::load_val_bytes(manifest)
}

/// Enforce the F-S1.04 device profile, then load and sha-verify train/val bytes.
pub fn verified_corpus(manifest: &TinyStoriesManifest) -> Result<VerifiedCorpus, S1ManifestError> {
    let enforcement = enforce(&S1CpuDeterministic::canonical())?;
    verified_corpus_after_enforcement(manifest, enforcement)
}

/// Load and sha-verify train/val bytes once the caller has already enforced F-S1.04.
pub fn verified_corpus_after_enforcement(
    manifest: &TinyStoriesManifest,
    _enforcement: DeviceProfileEnforcement,
) -> Result<VerifiedCorpus, S1ManifestError> {
    let train = load_train_bytes(manifest)?;
    let val = load_val_bytes(manifest)?;
    Ok(VerifiedCorpus { train, val })
}

/// Compute the canonical validation shuffle pin and emit manifest-pin evidence.
pub fn compute_val_shuffle_pin(val: &[u8]) -> Result<Hash256, S1ShufflePinError> {
    let shuffled = fisher_yates(val, NEGATIVE_TEST_SHUFFLE_SEED);
    let observed = sha256(&shuffled);
    S1LogEmitter::new().manifest_shuffle_pin_compute(&ManifestShufflePinComputeEvent {
        shuffle_seed: NEGATIVE_TEST_SHUFFLE_SEED,
        token_count: u64::try_from(val.len()).map_err(|_| S1ShufflePinError::LengthOverflow)?,
        shuffled_val_sha256: observed.to_string(),
    })?;
    Ok(observed)
}

/// Compute and verify the canonical validation shuffle pin.
pub fn verify_val_shuffle_pin(expected: Hash256, val: &[u8]) -> Result<Hash256, S1ShufflePinError> {
    let observed = compute_val_shuffle_pin(val)?;
    let emitter = S1LogEmitter::new();
    if observed == expected {
        emitter.manifest_shuffle_pin_verify_ok(&ManifestShufflePinVerifyOkEvent {
            expected: expected.to_string(),
            observed: observed.to_string(),
        })?;
        Ok(observed)
    } else {
        emitter.manifest_shuffle_pin_verify_fail(&ManifestShufflePinVerifyFailEvent {
            expected: expected.to_string(),
            observed: observed.to_string(),
        })?;
        Err(S1ShufflePinError::Mismatch { expected, observed })
    }
}

/// Errors returned by manifest shuffle-pin helpers.
#[derive(Debug)]
pub enum S1ShufflePinError {
    /// The validation byte slice length did not fit in the logging field.
    LengthOverflow,
    /// The computed shuffle hash did not match the manifest pin.
    Mismatch {
        /// Manifest-pinned hash.
        expected: Hash256,
        /// Computed shuffled validation hash.
        observed: Hash256,
    },
    /// Structured logging event construction failed.
    Logging(LoggingEventError),
}

impl fmt::Display for S1ShufflePinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthOverflow => f.write_str("validation byte length does not fit in u64"),
            Self::Mismatch { expected, observed } => write!(
                f,
                "TinyStories validation shuffle pin mismatch: expected {expected}, observed {observed}"
            ),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S1ShufflePinError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Logging(error) => Some(error),
            Self::LengthOverflow | Self::Mismatch { .. } => None,
        }
    }
}

impl From<LoggingEventError> for S1ShufflePinError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

/// Errors returned while enforcing S1 preconditions and verifying corpus bytes.
#[derive(Debug)]
pub enum S1ManifestError {
    /// F-S1.04 deterministic device-profile enforcement failed.
    DeviceProfile(DeviceProfileEnforceError),
    /// Manifest parsing, file I/O, byte length, or sha256 verification failed.
    Corpus(CorpusManifestError),
}

impl fmt::Display for S1ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceProfile(error) => write!(f, "{error}"),
            Self::Corpus(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S1ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DeviceProfile(error) => Some(error),
            Self::Corpus(error) => Some(error),
        }
    }
}

impl From<DeviceProfileEnforceError> for S1ManifestError {
    fn from(error: DeviceProfileEnforceError) -> Self {
        Self::DeviceProfile(error)
    }
}

impl From<CorpusManifestError> for S1ManifestError {
    fn from(error: CorpusManifestError) -> Self {
        Self::Corpus(error)
    }
}
