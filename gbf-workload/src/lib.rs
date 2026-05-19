//! Workload manifests, prompt suites, observation policies, and acceptance matrices.

/// Compile-time marker that the F-S3 workload schema surface is enabled.
#[cfg(feature = "s3-schemas")]
pub const S3_SCHEMAS_FEATURE_ENABLED: bool = true;

pub mod manifest;
pub mod matrix;
pub mod observation;
pub mod prompts;

pub use manifest::*;
