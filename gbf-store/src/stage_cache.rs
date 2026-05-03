//! Deterministic two-component stage-cache keys over the blob store.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use gbf_foundation::{Hash256, SemVer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;

use crate::blob::{BlobStore, BlobStoreError};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn from_static(value: &'static str) -> Self {
                Self(value.to_owned())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_id!(StageId);
string_id!(ComponentId);
string_id!(FeatureFlag);

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ComponentDigestSet {
    pub components: BTreeMap<ComponentId, Hash256>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StageKey {
    pub stage_id: StageId,
    pub shard_local: ComponentDigestSet,
    pub global: Hash256,
    pub feature_flags: BTreeSet<FeatureFlag>,
    pub pass_version: SemVer,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct StageCacheKey(Hash256);

impl fmt::Display for StageCacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StageCacheEntry {
    pub key: StageCacheKey,
    pub payload_hash: Hash256,
}

pub struct StageCache<'s> {
    blob: &'s BlobStore,
}

impl<'s> StageCache<'s> {
    #[must_use]
    pub fn new(blob: &'s BlobStore) -> Self {
        Self { blob }
    }

    pub fn get(&self, key: &StageKey) -> Result<Option<Vec<u8>>, StageCacheError> {
        let composed = try_compose_key(key)?;
        let index_path = self.index_path_for(composed);

        let payload_hash = match read_index_hash(&index_path) {
            Ok(hash) => hash,
            Err(StageCacheError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        match self.blob.get(payload_hash) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(BlobStoreError::NotFound { .. }) => Ok(None),
            Err(err) => Err(StageCacheError::BlobStore(err)),
        }
    }

    pub fn put(&self, key: &StageKey, payload: &[u8]) -> Result<StageCacheEntry, StageCacheError> {
        let composed = try_compose_key(key)?;
        let payload_hash = self.blob.put(payload)?;
        atomic_write_index(&self.index_path_for(composed), payload_hash)?;
        Ok(StageCacheEntry {
            key: composed,
            payload_hash,
        })
    }

    #[must_use]
    pub fn index_path_for(&self, key: StageCacheKey) -> PathBuf {
        cache_index_path(self.blob, key)
    }
}

/// Compose a structured stage key into an opaque cache-index key.
///
/// ```compile_fail
/// use gbf_store::blob::BlobStore;
/// use gbf_store::stage_cache::{compose_key, StageKey};
///
/// fn cache_key_is_not_a_blob_hash(store: &BlobStore, key: &StageKey) {
///     let _ = store.get(compose_key(key));
/// }
/// ```
#[must_use]
pub fn compose_key(key: &StageKey) -> StageCacheKey {
    try_compose_key(key).expect("stage-cache SemVer components must fit canonical u32 encoding")
}

pub fn try_compose_key(key: &StageKey) -> Result<StageCacheKey, StageCacheError> {
    let mut hasher = Sha256::new();

    update_string(&mut hasher, key.stage_id.as_str())?;
    hasher.update(
        len_u32(
            "shard_local component count",
            key.shard_local.components.len(),
        )?
        .to_le_bytes(),
    );
    for (component, hash) in &key.shard_local.components {
        update_string(&mut hasher, component.as_str())?;
        hasher.update(hash.as_bytes());
    }

    hasher.update(key.global.as_bytes());
    hasher.update(len_u32("feature flag count", key.feature_flags.len())?.to_le_bytes());
    for flag in &key.feature_flags {
        update_string(&mut hasher, flag.as_str())?;
    }

    hasher.update(semver_u32("major", key.pass_version.major)?.to_le_bytes());
    hasher.update(semver_u32("minor", key.pass_version.minor)?.to_le_bytes());
    hasher.update(semver_u32("patch", key.pass_version.patch)?.to_le_bytes());

    Ok(StageCacheKey(Hash256::from_bytes(hasher.finalize().into())))
}

#[derive(Debug)]
pub enum StageCacheError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    KeyEncodingFailed { component: &'static str, value: u64 },
    InvalidIndex { path: PathBuf, detail: String },
}

impl fmt::Display for StageCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "stage cache I/O error: {err}"),
            Self::BlobStore(err) => write!(f, "stage cache blob-store error: {err}"),
            Self::KeyEncodingFailed { component, value } => {
                write!(
                    f,
                    "stage cache {component} value {value} exceeds canonical u32 encoding"
                )
            }
            Self::InvalidIndex { path, detail } => {
                write!(f, "invalid stage-cache index {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for StageCacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::BlobStore(err) => Some(err),
            Self::KeyEncodingFailed { .. } | Self::InvalidIndex { .. } => None,
        }
    }
}

impl From<io::Error> for StageCacheError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<BlobStoreError> for StageCacheError {
    fn from(value: BlobStoreError) -> Self {
        Self::BlobStore(value)
    }
}

pub(crate) fn cache_index_root(store: &BlobStore) -> PathBuf {
    store.root().join("cache")
}

pub(crate) fn cache_index_path(store: &BlobStore, key: StageCacheKey) -> PathBuf {
    let hex = key.to_string();
    cache_index_root(store).join(&hex[..2]).join(hex)
}

pub(crate) fn read_index_hash(path: &Path) -> Result<Hash256, StageCacheError> {
    let raw = fs::read_to_string(path)?;
    let trimmed = raw.trim();
    trimmed
        .parse()
        .map_err(
            |err: gbf_foundation::Hash256ParseError| StageCacheError::InvalidIndex {
                path: path.to_path_buf(),
                detail: err.to_string(),
            },
        )
}

fn atomic_write_index(path: &Path, payload_hash: Hash256) -> Result<(), StageCacheError> {
    // Index files are weak cache references: a missing or stale index is a
    // cache miss, so the blob store's `DurabilityMode::Full` directory sync is
    // intentionally not mirrored here.
    let parent = path
        .parent()
        .expect("stage-cache index path always has a shard parent");
    fs::create_dir_all(parent)?;
    let mut tmp = Builder::new()
        .prefix("stage-index-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    write!(tmp, "{payload_hash}")?;
    tmp.as_file_mut().sync_all()?;
    let (_file, tmp_path) = tmp.keep().map_err(|err| err.error)?;
    match fs::rename(&tmp_path, path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            if let Err(err) = fs::remove_file(path).and_then(|_| fs::rename(&tmp_path, path)) {
                let _ = fs::remove_file(&tmp_path);
                return Err(StageCacheError::Io(err));
            }
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp_path);
            Err(StageCacheError::Io(err))
        }
    }
}

fn update_string(hasher: &mut Sha256, value: &str) -> Result<(), StageCacheError> {
    let bytes = value.as_bytes();
    hasher.update(len_u32("string length", bytes.len())?.to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn semver_u32(component: &'static str, value: u64) -> Result<u32, StageCacheError> {
    u32::try_from(value).map_err(|_| StageCacheError::KeyEncodingFailed { component, value })
}

fn len_u32(component: &'static str, value: usize) -> Result<u32, StageCacheError> {
    u32::try_from(value).map_err(|_| StageCacheError::KeyEncodingFailed {
        component,
        value: value as u64,
    })
}
