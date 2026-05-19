//! S3 reproducibility environment hash surface.

use std::sync::OnceLock;

use gbf_foundation::{Hash256, sha256};
use serde::Serialize;

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
))]
use crate::s1::schema::{DomainHash, S1SchemaError};

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
)))]
use gbf_foundation::DomainHash;

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "falsify"
)))]
/// Schema-only error alias used when S1 modules are not compiled.
pub type S1SchemaError = gbf_foundation::CanonicalJsonError;

/// Reproducibility inputs outside the semantic S3 train/workload config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S3EnvironmentHash {
    /// Hash of the active Cargo feature/build configuration.
    pub build_config_hash: Hash256,
    /// Hash of the Rust toolchain identity compiled into this crate.
    pub rust_toolchain_hash: Hash256,
    /// Hash of the dependency lockfile bytes compiled into this crate.
    pub dependency_lockfile_hash: Hash256,
    /// Hash of the real/fallback oracle backend identity selected for this run.
    pub oracle_backend_identity: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct S3BuildConfigIdentity<'a> {
    package: &'a str,
    active_features: Vec<&'static str>,
    cfg_debug_assertions: bool,
    target_arch: &'a str,
    target_os: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct S3ToolchainIdentity<'a> {
    rustc_version: &'a str,
    package_rust_version: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct S3OracleBackendIdentity<'a> {
    denotational_backend: &'a str,
    artifact_backend: &'a str,
    adversarial_enabled: bool,
}

/// Compute the S3 reproducibility hashes for the active binary.
pub fn compute_environment_hash() -> Result<S3EnvironmentHash, S1SchemaError> {
    static CACHE: OnceLock<S3EnvironmentHash> = OnceLock::new();
    if let Some(hash) = CACHE.get() {
        return Ok(*hash);
    }
    let hash = environment_hash_for_inputs(
        active_features(),
        env!("GBF_RUSTC_VERSION"),
        env!("CARGO_PKG_RUST_VERSION"),
        include_bytes!("../../../Cargo.lock"),
        oracle_backend_identity_hash_for_active_features()?,
    )?;
    let _ = CACHE.set(hash);
    Ok(hash)
}

/// Compute the full environment hash surface for explicit, uncached inputs.
pub fn environment_hash_for_inputs(
    active_features: Vec<&'static str>,
    rustc_version: &'static str,
    package_rust_version: &'static str,
    lockfile_bytes: impl AsRef<[u8]>,
    oracle_backend_identity: Hash256,
) -> Result<S3EnvironmentHash, S1SchemaError> {
    Ok(S3EnvironmentHash {
        build_config_hash: build_config_hash_for_features(active_features)?,
        rust_toolchain_hash: rust_toolchain_hash_for_identity(rustc_version, package_rust_version),
        dependency_lockfile_hash: dependency_lockfile_hash_for_bytes(lockfile_bytes),
        oracle_backend_identity,
    })
}

/// Compute the build-config hash for an explicit feature list.
pub fn build_config_hash_for_features(
    active_features: Vec<&'static str>,
) -> Result<Hash256, S1SchemaError> {
    let identity = S3BuildConfigIdentity {
        package: "gbf-experiments",
        active_features,
        cfg_debug_assertions: cfg!(debug_assertions),
        target_arch: std::env::consts::ARCH,
        target_os: std::env::consts::OS,
    };
    DomainHash::new(
        "gbf-experiments",
        "S3BuildConfigIdentity",
        "s3_build_config.v1",
        "1",
    )
    .hash(&identity)
}

/// Compute the dependency lockfile hash for supplied bytes.
#[must_use]
pub fn dependency_lockfile_hash_for_bytes(bytes: impl AsRef<[u8]>) -> Hash256 {
    sha256(bytes)
}

/// Compute the Rust toolchain identity hash for explicit pure inputs.
#[must_use]
pub fn rust_toolchain_hash_for_identity(
    rustc_version: &'static str,
    package_rust_version: &'static str,
) -> Hash256 {
    let identity = S3ToolchainIdentity {
        rustc_version,
        package_rust_version,
    };
    let bytes = serde_json::to_vec(&identity)
        .expect("S3ToolchainIdentity serialization should be infallible");
    sha256(bytes)
}

/// Compute the oracle-backend identity hash for explicit pure inputs.
pub fn oracle_backend_identity_hash_for_inputs(
    denotational_backend: &'static str,
    artifact_backend: &'static str,
    adversarial_enabled: bool,
) -> Result<Hash256, S1SchemaError> {
    let identity = S3OracleBackendIdentity {
        denotational_backend,
        artifact_backend,
        adversarial_enabled,
    };
    DomainHash::new(
        "gbf-experiments",
        "S3OracleBackendIdentity",
        "s3_oracle_backend_identity.v1",
        "1",
    )
    .hash(&identity)
}

fn oracle_backend_identity_hash_for_active_features() -> Result<Hash256, S1SchemaError> {
    let (denotational_backend, artifact_backend) = if cfg!(feature = "s3-oracle-real") {
        (
            "gbf-oracle::DenotationalOracle",
            "gbf-oracle::ArtifactOracle",
        )
    } else if cfg!(feature = "s3-oracle-fallback") {
        (
            "gbf-experiments::s3::oracle::S3DenotationalFallback",
            "gbf-experiments::s3::oracle::S3ArtifactFallback",
        )
    } else {
        ("unselected", "unselected")
    };
    oracle_backend_identity_hash_for_inputs(
        denotational_backend,
        artifact_backend,
        cfg!(feature = "s3-oracle-adversarial"),
    )
}

fn active_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    if cfg!(feature = "phase-a") {
        features.push("phase-a");
    }
    if cfg!(feature = "ablation") {
        features.push("ablation");
    }
    if cfg!(feature = "s2-full") {
        features.push("s2-full");
    }
    if cfg!(feature = "s2-ablation") {
        features.push("s2-ablation");
    }
    if cfg!(feature = "s3") {
        features.push("s3");
    }
    if cfg!(feature = "s3-phase-d") {
        features.push("s3-phase-d");
    }
    if cfg!(feature = "s3-oracle-real") {
        features.push("s3-oracle-real");
    }
    if cfg!(feature = "s3-oracle-fallback") {
        features.push("s3-oracle-fallback");
    }
    if cfg!(feature = "s3-oracle-adversarial") {
        features.push("s3-oracle-adversarial");
    }
    if cfg!(feature = "falsify") {
        features.push("falsify");
    }
    features
}
