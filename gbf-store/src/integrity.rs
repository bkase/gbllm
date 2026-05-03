//! Store integrity verification.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::io;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::blob::{BlobStore, BlobStoreError, hash_bytes};
use crate::gc::{BlobReferenceError, BlobReferencesRegistry};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub blobs_checked: u64,
    pub mismatches: Vec<Hash256>,
    pub missing: Vec<Hash256>,
}

pub fn verify_integrity(store: &BlobStore, hash: Hash256) -> Result<(), IntegrityError> {
    let bytes = match store.get(hash) {
        Ok(bytes) => bytes,
        Err(BlobStoreError::NotFound { hash }) => return Err(IntegrityError::NotFound { hash }),
        Err(err) => return Err(IntegrityError::BlobStore(err)),
    };
    let actual = hash_bytes(&bytes);
    if actual != hash {
        return Err(IntegrityError::HashMismatch {
            expected: hash,
            actual,
        });
    }
    Ok(())
}

pub fn verify_all(store: &BlobStore) -> Result<IntegrityReport, IntegrityError> {
    let mut report = IntegrityReport {
        blobs_checked: 0,
        mismatches: Vec::new(),
        missing: Vec::new(),
    };

    for hash in store.list_blobs()? {
        report.blobs_checked += 1;
        match verify_integrity(store, hash) {
            Ok(()) => {}
            Err(IntegrityError::HashMismatch { expected, .. }) => {
                report.mismatches.push(expected);
            }
            Err(err) => return Err(err),
        }
    }

    Ok(report)
}

pub fn verify_reachable(
    store: &BlobStore,
    roots: impl IntoIterator<Item = Hash256>,
    refs: &BlobReferencesRegistry,
) -> Result<IntegrityReport, IntegrityError> {
    let mut report = IntegrityReport {
        blobs_checked: 0,
        mismatches: Vec::new(),
        missing: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();

    for root in roots {
        if seen.insert(root) {
            queue.push_back(root);
        }
    }

    while let Some(hash) = queue.pop_front() {
        let bytes = match store.get(hash) {
            Ok(bytes) => bytes,
            Err(BlobStoreError::NotFound { hash }) => {
                report.missing.push(hash);
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        report.blobs_checked += 1;
        let actual = hash_bytes(&bytes);
        if actual != hash {
            report.mismatches.push(hash);
            continue;
        }
        if let Some(children) = refs.referenced_blobs(hash, &bytes)? {
            for child in children {
                if seen.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    Ok(report)
}

#[derive(Debug)]
pub enum IntegrityError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    HashMismatch { expected: Hash256, actual: Hash256 },
    NotFound { hash: Hash256 },
    BlobReferenceDecode(BlobReferenceError),
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "integrity I/O error: {err}"),
            Self::BlobStore(err) => write!(f, "integrity blob-store error: {err}"),
            Self::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "integrity hash mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NotFound { hash } => write!(f, "integrity check could not find blob {hash}"),
            Self::BlobReferenceDecode(err) => write!(f, "blob reference decode failed: {err}"),
        }
    }
}

impl std::error::Error for IntegrityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::BlobStore(err) => Some(err),
            Self::BlobReferenceDecode(err) => Some(err),
            Self::HashMismatch { .. } | Self::NotFound { .. } => None,
        }
    }
}

impl From<io::Error> for IntegrityError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<BlobStoreError> for IntegrityError {
    fn from(value: BlobStoreError) -> Self {
        Self::BlobStore(value)
    }
}

impl From<BlobReferenceError> for IntegrityError {
    fn from(value: BlobReferenceError) -> Self {
        Self::BlobReferenceDecode(value)
    }
}
