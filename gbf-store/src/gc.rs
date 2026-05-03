//! Pinset-driven garbage collection.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::blob::{BlobStore, BlobStoreError};
use crate::pinset::Pinset;
use crate::stage_cache::{cache_index_root, read_index_hash};

#[derive(Clone, Debug)]
pub struct GcOptions {
    /// Report candidates without removing blobs or stage-cache indexes.
    pub dry_run: bool,
    /// Cap the number of blobs removed in this run after deterministic sorting.
    pub max_remove_per_run: Option<usize>,
    /// Remove stage-cache index files for payload blobs removed in this run.
    pub sweep_stage_cache_indexes: bool,
    /// Policy for pinned blobs that no registered byte recognizer can decode.
    pub unknown_reference_policy: UnknownReferencePolicy,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            max_remove_per_run: None,
            sweep_stage_cache_indexes: false,
            unknown_reference_policy: UnknownReferencePolicy::Abort,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UnknownReferencePolicy {
    /// Default. Refuse to GC if a pinned blob may be reference-bearing but no
    /// registered reader recognizes it.
    Abort,
    /// Treat undecodable blobs as leaves. Children reachable only through that
    /// parent may be removed.
    TreatAsLeaf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GcReport {
    pub pinsets_walked: u64,
    pub blobs_kept: u64,
    pub candidate_blobs: u64,
    pub candidate_bytes: u64,
    pub blobs_removed: u64,
    pub bytes_freed: u64,
    pub removed: Vec<Hash256>,
}

pub struct BlobReferencesRegistry {
    readers: Vec<Box<dyn BlobReferenceReader>>,
}

impl BlobReferencesRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            readers: Vec::new(),
        }
    }

    /// Alias for `new`, useful at call sites that intentionally run without
    /// any reference recognizers.
    #[must_use]
    pub fn empty() -> Self {
        Self::new()
    }

    pub fn register<R>(&mut self, reader: R)
    where
        R: BlobReferenceReader + 'static,
    {
        self.readers.push(Box::new(reader));
    }

    pub fn referenced_blobs(
        &self,
        hash: Hash256,
        bytes: &[u8],
    ) -> Result<Option<Vec<Hash256>>, BlobReferenceError> {
        for reader in &self.readers {
            if let Some(refs) = reader.referenced_blobs(hash, bytes)? {
                return Ok(Some(refs));
            }
        }
        Ok(None)
    }
}

impl Default for BlobReferencesRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub trait BlobReferenceReader: Send + Sync {
    fn referenced_blobs(
        &self,
        hash: Hash256,
        bytes: &[u8],
    ) -> Result<Option<Vec<Hash256>>, BlobReferenceError>;
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BlobReferenceError {
    Decode(String),
}

impl fmt::Display for BlobReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(detail) => write!(f, "failed to decode blob references: {detail}"),
        }
    }
}

impl std::error::Error for BlobReferenceError {}

pub fn run_gc(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
    opts: &GcOptions,
) -> Result<GcReport, GcError> {
    let keep = collect_keep_set(store, pinsets, refs, opts.unknown_reference_policy)?;
    let all = store.list_blobs()?;
    let mut candidates: Vec<(Hash256, u64)> = Vec::new();
    let mut candidate_bytes = 0_u64;

    for hash in all {
        if !keep.contains(&hash) {
            let len = match blob_len(store, hash) {
                Ok(len) => len,
                Err(GcError::BlobStore(BlobStoreError::NotFound { .. })) => continue,
                Err(err) => return Err(err),
            };
            candidate_bytes = candidate_bytes.saturating_add(len);
            candidates.push((hash, len));
        }
    }
    candidates.sort_by_key(|(hash, _)| *hash);

    let mut report = GcReport {
        pinsets_walked: pinsets.len() as u64,
        blobs_kept: keep.len() as u64,
        candidate_blobs: candidates.len() as u64,
        candidate_bytes,
        blobs_removed: 0,
        bytes_freed: 0,
        removed: Vec::new(),
    };

    if opts.dry_run {
        return Ok(report);
    }

    let remove_limit = opts.max_remove_per_run.unwrap_or(candidates.len());
    for (hash, len) in candidates.iter().copied().take(remove_limit) {
        store.remove(hash)?;
        report.blobs_removed += 1;
        report.bytes_freed = report.bytes_freed.saturating_add(len);
        report.removed.push(hash);
    }

    if opts.sweep_stage_cache_indexes {
        let removed_set: BTreeSet<Hash256> = report.removed.iter().copied().collect();
        sweep_stage_cache_indexes(store, &removed_set)?;
    }

    Ok(report)
}

fn collect_keep_set(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
    unknown_policy: UnknownReferencePolicy,
) -> Result<BTreeSet<Hash256>, GcError> {
    let mut keep = BTreeSet::new();
    let mut queue = VecDeque::new();

    for pinset in pinsets {
        for root in &pinset.roots {
            if keep.insert(*root) {
                queue.push_back(*root);
            }
        }
    }

    while let Some(hash) = queue.pop_front() {
        let bytes = store.get(hash)?;
        match refs.referenced_blobs(hash, &bytes)? {
            Some(children) => {
                for child in children {
                    if keep.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
            None if unknown_policy == UnknownReferencePolicy::TreatAsLeaf => {}
            None => return Err(GcError::UndecodableReferenceBearingBlob { hash }),
        }
    }

    Ok(keep)
}

fn blob_len(store: &BlobStore, hash: Hash256) -> Result<u64, GcError> {
    match fs::metadata(store.path_for(hash)) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            Err(GcError::BlobStore(BlobStoreError::NotFound { hash }))
        }
        Err(err) => Err(GcError::Io(err)),
    }
}

fn sweep_stage_cache_indexes(
    store: &BlobStore,
    removed_set: &BTreeSet<Hash256>,
) -> Result<(), GcError> {
    let root = cache_index_root(store);
    if !root.exists() {
        return Ok(());
    }

    for shard in fs::read_dir(root)? {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(shard.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(payload_hash) = read_index_hash(&path) else {
                continue;
            };
            if removed_set.contains(&payload_hash) {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(GcError::Io(err)),
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
pub enum GcError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    BlobReferenceDecode(BlobReferenceError),
    UndecodableReferenceBearingBlob { hash: Hash256 },
}

impl fmt::Display for GcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "GC I/O error: {err}"),
            Self::BlobStore(err) => write!(f, "GC blob-store error: {err}"),
            Self::BlobReferenceDecode(err) => write!(f, "GC blob-reference decode error: {err}"),
            Self::UndecodableReferenceBearingBlob { hash } => {
                write!(f, "GC cannot decode references for pinned blob {hash}")
            }
        }
    }
}

impl std::error::Error for GcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::BlobStore(err) => Some(err),
            Self::BlobReferenceDecode(err) => Some(err),
            Self::UndecodableReferenceBearingBlob { .. } => None,
        }
    }
}

impl From<io::Error> for GcError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<BlobStoreError> for GcError {
    fn from(value: BlobStoreError) -> Self {
        Self::BlobStore(value)
    }
}

impl From<BlobReferenceError> for GcError {
    fn from(value: BlobReferenceError) -> Self {
        Self::BlobReferenceDecode(value)
    }
}
