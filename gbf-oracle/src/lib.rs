//! Executable specifications for denotational, artifact, and scheduled execution semantics.

/// Compile-time marker that the real F-S3 oracle backend is enabled.
#[cfg(feature = "s3-real")]
pub const S3_REAL_FEATURE_ENABLED: bool = true;

/// Compile-time marker that the fallback F-S3 oracle backend is enabled.
#[cfg(feature = "s3-fallback")]
pub const S3_FALLBACK_FEATURE_ENABLED: bool = true;

pub mod artifact;
pub mod conformance;
pub mod denotational;
pub mod phase_surface_agreement;
pub mod schedule;
pub mod scorers;
