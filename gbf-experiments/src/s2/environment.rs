//! S2 reproducibility environment hash surface.

use std::sync::OnceLock;

use gbf_foundation::{Hash256, sha256};
use serde::Serialize;

use crate::s1::schema::{DomainHash, S1SchemaError};

/// Reproducibility inputs outside the semantic train config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S2EnvironmentHash {
    /// Hash of the active Cargo feature/build configuration.
    pub build_config_hash: Hash256,
    /// Hash of the Rust toolchain identity compiled into this crate.
    pub rust_toolchain_hash: Hash256,
    /// Hash of the dependency lockfile bytes compiled into this crate.
    pub dependency_lockfile_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct S2BuildConfigIdentity<'a> {
    package: &'a str,
    active_features: Vec<&'static str>,
    cfg_debug_assertions: bool,
    target_arch: &'a str,
    target_os: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct S2ToolchainIdentity<'a> {
    rustc_version: &'a str,
    package_rust_version: &'a str,
}

/// Compute the S2 reproducibility hashes for the active binary.
pub fn compute_environment_hash() -> Result<S2EnvironmentHash, S1SchemaError> {
    static CACHE: OnceLock<S2EnvironmentHash> = OnceLock::new();
    if let Some(hash) = CACHE.get() {
        return Ok(*hash);
    }
    let hash = environment_hash_for_inputs(
        active_features(),
        env!("GBF_RUSTC_VERSION"),
        env!("CARGO_PKG_RUST_VERSION"),
        // `environment.rs` lives under gbf-experiments/src/s2, so this
        // path walks back to the workspace root Cargo.lock, not a crate
        // local lockfile.
        include_bytes!("../../../Cargo.lock"),
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
) -> Result<S2EnvironmentHash, S1SchemaError> {
    Ok(S2EnvironmentHash {
        build_config_hash: build_config_hash_for_features(active_features)?,
        rust_toolchain_hash: rust_toolchain_hash_for_identity(rustc_version, package_rust_version),
        dependency_lockfile_hash: dependency_lockfile_hash_for_bytes(lockfile_bytes),
    })
}

/// Compute the build-config hash for an explicit feature list.
///
/// This is public so integration tests can prove feature sensitivity without
/// having to spawn separate Cargo builds.
pub fn build_config_hash_for_features(
    active_features: Vec<&'static str>,
) -> Result<Hash256, S1SchemaError> {
    let identity = S2BuildConfigIdentity {
        package: "gbf-experiments",
        active_features,
        cfg_debug_assertions: cfg!(debug_assertions),
        target_arch: std::env::consts::ARCH,
        target_os: std::env::consts::OS,
    };
    DomainHash::new(
        "gbf-experiments",
        "S2BuildConfigIdentity",
        "s2_build_config.v1",
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
    let identity = S2ToolchainIdentity {
        rustc_version,
        package_rust_version,
    };
    let bytes = serde_json::to_vec(&identity)
        .expect("S2ToolchainIdentity serialization should be infallible");
    sha256(bytes)
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
    if cfg!(feature = "falsify") {
        features.push("falsify");
    }
    features
}
