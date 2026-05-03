//! Content-addressed blob storage with atomic writes.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gbf_foundation::{BlobCodec, BlobRef, Hash256};
use sha2::{Digest, Sha256};
use tempfile::Builder;

/// Content-addressed storage rooted at `blobs/sha256/<ab>/<hash>`.
#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
    durability: DurabilityMode,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum DurabilityMode {
    /// Fsync the temp file before rename. This is enough for read-after-return
    /// on the same machine, but not a full power-loss directory-entry guarantee.
    #[default]
    ReadAfterReturn,
    /// Also best-effort fsync the destination directory after rename.
    Full,
}

impl BlobStore {
    pub fn open(root: PathBuf) -> Result<Self, BlobStoreError> {
        Self::open_with_durability(root, DurabilityMode::default())
    }

    pub fn open_with_durability(
        root: PathBuf,
        durability: DurabilityMode,
    ) -> Result<Self, BlobStoreError> {
        fs::create_dir_all(root.join("blobs").join("sha256"))?;
        fs::create_dir_all(root.join("tmp"))?;
        Ok(Self { root, durability })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put(&self, bytes: &[u8]) -> Result<Hash256, BlobStoreError> {
        let hash = hash_bytes(bytes);
        self.commit_bytes(hash, bytes)?;
        Ok(hash)
    }

    pub fn put_expect(&self, expected: Hash256, bytes: &[u8]) -> Result<Hash256, BlobStoreError> {
        let actual = hash_bytes(bytes);
        if actual != expected {
            return Err(BlobStoreError::HashMismatch { expected, actual });
        }
        self.commit_bytes(expected, bytes)?;
        Ok(expected)
    }

    pub fn put_as(&self, bytes: &[u8], codec: BlobCodec) -> Result<BlobRef, BlobStoreError> {
        let len = checked_blob_len(bytes.len())?;
        let hash = self.put(bytes)?;
        Ok(BlobRef { hash, len, codec })
    }

    pub fn put_streaming<R: Read>(&self, mut reader: R) -> Result<Hash256, BlobStoreError> {
        let mut tmp = self.tmp_file()?;
        let mut hasher = Sha256::new();
        let mut buf = [0_u8; 64 * 1024];

        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
            tmp.write_all(&buf[..read])?;
        }

        tmp.as_file_mut().sync_all()?;
        let hash = Hash256::from_bytes(hasher.finalize().into());
        let (_file, tmp_path) = tmp.keep().map_err(|err| err.error)?;
        self.commit_tmp(hash, &tmp_path)?;
        Ok(hash)
    }

    pub fn get(&self, hash: Hash256) -> Result<Vec<u8>, BlobStoreError> {
        match fs::read(self.path_for(hash)) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(BlobStoreError::NotFound { hash })
            }
            Err(err) => Err(BlobStoreError::Io(err)),
        }
    }

    pub fn get_streaming(&self, hash: Hash256) -> Result<BufReader<File>, BlobStoreError> {
        match File::open(self.path_for(hash)) {
            Ok(file) => Ok(BufReader::new(file)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(BlobStoreError::NotFound { hash })
            }
            Err(err) => Err(BlobStoreError::Io(err)),
        }
    }

    pub fn get_ref(&self, blob_ref: BlobRef) -> Result<Vec<u8>, BlobStoreError> {
        let bytes = self.get(blob_ref.hash)?;
        let actual = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        if actual != blob_ref.len {
            return Err(BlobStoreError::LenMismatch {
                expected: blob_ref.len,
                actual,
            });
        }
        Ok(bytes)
    }

    #[must_use]
    pub fn exists(&self, hash: Hash256) -> bool {
        self.path_for(hash).exists()
    }

    pub fn remove(&self, hash: Hash256) -> Result<(), BlobStoreError> {
        match fs::remove_file(self.path_for(hash)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(BlobStoreError::Io(err)),
        }
    }

    pub fn list_blobs(&self) -> Result<Vec<Hash256>, BlobStoreError> {
        let root = self.sha256_root();
        if !root.exists() {
            return Ok(Vec::new());
        }

        let mut hashes = Vec::new();
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
                let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                    continue;
                };
                if name.len() == 64
                    && let Ok(hash) = name.parse::<Hash256>()
                {
                    hashes.push(hash);
                }
            }
        }
        hashes.sort();
        hashes.dedup();
        Ok(hashes)
    }

    #[must_use]
    pub fn path_for(&self, hash: Hash256) -> PathBuf {
        let hex = hash.to_string();
        self.sha256_root().join(&hex[..2]).join(hex)
    }

    pub fn cleanup_tmp(&self, max_age: Duration) -> Result<u32, BlobStoreError> {
        let mut removed = 0_u32;
        for entry in fs::read_dir(self.tmp_root())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let metadata = entry.metadata()?;
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let Ok(age) = modified.elapsed() else {
                continue;
            };
            if age >= max_age {
                fs::remove_file(entry.path())?;
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }

    fn commit_bytes(&self, hash: Hash256, bytes: &[u8]) -> Result<(), BlobStoreError> {
        let canonical = self.path_for(hash);
        if canonical.exists() {
            self.verify_existing(hash)?;
            return Ok(());
        }

        let mut tmp = self.tmp_file()?;
        tmp.write_all(bytes)?;
        tmp.as_file_mut().sync_all()?;
        let (_file, tmp_path) = tmp.keep().map_err(|err| err.error)?;
        self.commit_tmp(hash, &tmp_path)
    }

    fn commit_tmp(&self, hash: Hash256, tmp_path: &Path) -> Result<(), BlobStoreError> {
        let canonical = self.path_for(hash);
        if canonical.exists() {
            self.verify_existing(hash)?;
            remove_tmp_best_effort(tmp_path);
            return Ok(());
        }

        let parent = canonical
            .parent()
            .expect("canonical blob path always has a shard parent");
        fs::create_dir_all(parent)?;

        match fs::rename(tmp_path, &canonical) {
            Ok(()) => {
                if self.durability == DurabilityMode::Full {
                    sync_dir_best_effort(parent);
                }
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                self.verify_existing(hash)?;
                remove_tmp_best_effort(tmp_path);
                Ok(())
            }
            Err(err) => {
                remove_tmp_best_effort(tmp_path);
                Err(BlobStoreError::Io(err))
            }
        }
    }

    fn verify_existing(&self, hash: Hash256) -> Result<(), BlobStoreError> {
        let mut file = File::open(self.path_for(hash))?;
        let mut hasher = Sha256::new();
        let mut buf = [0_u8; 64 * 1024];

        loop {
            let read = file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }

        let actual = Hash256::from_bytes(hasher.finalize().into());
        if actual != hash {
            return Err(BlobStoreError::ExistingBlobCorrupt { hash, actual });
        }
        Ok(())
    }

    fn tmp_file(&self) -> Result<tempfile::NamedTempFile, BlobStoreError> {
        Ok(Builder::new()
            .prefix("blob-")
            .suffix(".tmp")
            .tempfile_in(self.tmp_root())?)
    }

    fn sha256_root(&self) -> PathBuf {
        self.root.join("blobs").join("sha256")
    }

    fn tmp_root(&self) -> PathBuf {
        self.root.join("tmp")
    }
}

#[derive(Debug)]
pub enum BlobStoreError {
    Io(io::Error),
    LenMismatch { expected: u32, actual: u32 },
    HashMismatch { expected: Hash256, actual: Hash256 },
    NotFound { hash: Hash256 },
    BlobTooLarge { len: u64, max: u64 },
    ExistingBlobCorrupt { hash: Hash256, actual: Hash256 },
}

impl fmt::Display for BlobStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "blob store I/O error: {err}"),
            Self::LenMismatch { expected, actual } => {
                write!(f, "blob length mismatch: expected {expected}, got {actual}")
            }
            Self::HashMismatch { expected, actual } => {
                write!(f, "blob hash mismatch: expected {expected}, got {actual}")
            }
            Self::NotFound { hash } => write!(f, "blob {hash} was not found"),
            Self::BlobTooLarge { len, max } => {
                write!(f, "blob length {len} exceeds maximum {max}")
            }
            Self::ExistingBlobCorrupt { hash, actual } => write!(
                f,
                "canonical blob path for {hash} contains bytes hashing to {actual}"
            ),
        }
    }
}

impl std::error::Error for BlobStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for BlobStoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes).into())
}

#[doc(hidden)]
pub fn checked_blob_len(len: usize) -> Result<u32, BlobStoreError> {
    u32::try_from(len).map_err(|_| BlobStoreError::BlobTooLarge {
        len: len as u64,
        max: u32::MAX as u64,
    })
}

fn remove_tmp_best_effort(tmp_path: &Path) {
    let _ = fs::remove_file(tmp_path);
}

fn sync_dir_best_effort(path: &Path) {
    let _ = File::open(path).and_then(|file| file.sync_all());
}
