//! Corpus manifests and raw-byte substrate verification.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const TINYSTORIES_MANIFEST_SCHEMA: &str = "tinystories_manifest.v1";

/// The S1 TinyStories raw-byte manifest checked into `fixtures/corpora/tinystories.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TinyStoriesManifest {
    #[serde(skip)]
    manifest_dir: Option<PathBuf>,
    pub schema: String,
    pub schema_version: String,
    pub corpus_id: String,
    pub dataset_version: String,
    pub source_name: String,
    pub source_url: String,
    pub train_path: String,
    pub val_path: String,
    pub train_sha256: Hash256,
    pub val_sha256: Hash256,
    #[serde(default)]
    pub val_shuffle_deadeef_sha256: Option<Hash256>,
    #[serde(default)]
    pub val_shuffle_deadeef_pinned_at_pass_version: Option<String>,
    pub source: CorpusSource,
    pub raw_root: String,
    pub raw_byte_policy: String,
    pub story_separator: String,
    pub splits: TinyStoriesSplits,
    pub s1_policy: String,
    pub deferred_scope: Vec<String>,
}

impl TinyStoriesManifest {
    pub fn from_toml_str(input: &str) -> Result<Self, CorpusManifestError> {
        let manifest: Self = toml::from_str(input).map_err(CorpusManifestError::Toml)?;
        manifest.validate_schema()?;
        Ok(manifest)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, CorpusManifestError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| CorpusManifestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let mut manifest = Self::from_toml_str(&text)?;
        manifest.manifest_dir = path.parent().map(Path::to_path_buf);
        Ok(manifest)
    }

    pub fn file(&self, split: SplitRole) -> &CorpusFile {
        match split {
            SplitRole::Train => &self.splits.train,
            SplitRole::Validation => &self.splits.validation,
        }
    }

    pub fn split_path(&self, split: SplitRole) -> PathBuf {
        let path = match split {
            SplitRole::Train => Path::new(&self.train_path),
            SplitRole::Validation => Path::new(&self.val_path),
        };
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(manifest_dir) = &self.manifest_dir {
            manifest_dir.join(path)
        } else {
            path.to_path_buf()
        }
    }

    fn validate_schema(&self) -> Result<(), CorpusManifestError> {
        if self.schema == TINYSTORIES_MANIFEST_SCHEMA {
            Ok(())
        } else {
            Err(CorpusManifestError::SchemaMismatch {
                expected: TINYSTORIES_MANIFEST_SCHEMA,
                observed: self.schema.clone(),
            })
        }
    }
}

pub fn read_tinystories_manifest(
    path: impl AsRef<Path>,
) -> Result<TinyStoriesManifest, CorpusManifestError> {
    TinyStoriesManifest::from_toml_file(path)
}

pub fn load_train_bytes(manifest: &TinyStoriesManifest) -> Result<Vec<u8>, CorpusManifestError> {
    load_split_bytes(manifest, SplitRole::Train)
}

pub fn load_val_bytes(manifest: &TinyStoriesManifest) -> Result<Vec<u8>, CorpusManifestError> {
    load_split_bytes(manifest, SplitRole::Validation)
}

fn load_split_bytes(
    manifest: &TinyStoriesManifest,
    split: SplitRole,
) -> Result<Vec<u8>, CorpusManifestError> {
    let path = manifest.split_path(split);
    let bytes = std::fs::read(&path).map_err(|source| CorpusManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    manifest.file(split).verify_bytes(&bytes)?;
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusSource {
    pub name: String,
    pub url: String,
    pub dataset_card_url: String,
    pub license: String,
    pub license_url: String,
    pub downloaded_at: String,
    pub decompression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TinyStoriesSplits {
    pub train: CorpusFile,
    pub validation: CorpusFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusFile {
    pub role: SplitRole,
    pub url: String,
    pub local_filename: String,
    pub sha256: Hash256,
    pub byte_length: u64,
    pub story_count: u64,
}

impl CorpusFile {
    /// Verify an already-loaded byte slice. Production verification of large
    /// corpus files should use [`Self::verify_file`] to avoid loading GiB-scale
    /// sources into memory.
    pub fn verify_bytes(&self, bytes: &[u8]) -> Result<(), CorpusManifestError> {
        let observed_len =
            u64::try_from(bytes.len()).map_err(|_| CorpusManifestError::LengthOverflow {
                path: self.local_filename.clone(),
            })?;
        if observed_len != self.byte_length {
            return Err(CorpusManifestError::ByteLengthMismatch {
                path: self.local_filename.clone(),
                expected: self.byte_length,
                observed: observed_len,
            });
        }

        let observed = gbf_foundation::sha256(bytes);
        if observed != self.sha256 {
            return Err(CorpusManifestError::Sha256Mismatch {
                path: self.local_filename.clone(),
                expected: self.sha256,
                observed,
            });
        }

        Ok(())
    }

    pub fn verify_file(&self, path: impl AsRef<Path>) -> Result<(), CorpusManifestError> {
        let path = path.as_ref();
        let mut file = File::open(path).map_err(|source| CorpusManifestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut len = 0_u64;
        let mut buffer = [0_u8; 1024 * 1024];

        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|source| CorpusManifestError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            if read == 0 {
                break;
            }
            let read_u64 = u64::try_from(read).expect("buffer read length always fits in u64");
            len = len
                .checked_add(read_u64)
                .ok_or_else(|| CorpusManifestError::LengthOverflow {
                    path: path.display().to_string(),
                })?;
            hasher.update(&buffer[..read]);
        }

        if len != self.byte_length {
            return Err(CorpusManifestError::ByteLengthMismatch {
                path: path.display().to_string(),
                expected: self.byte_length,
                observed: len,
            });
        }

        let observed = Hash256::from_bytes(hasher.finalize().into());
        if observed != self.sha256 {
            return Err(CorpusManifestError::Sha256Mismatch {
                path: path.display().to_string(),
                expected: self.sha256,
                observed,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitRole {
    Train,
    Validation,
}

#[derive(Debug)]
pub enum CorpusManifestError {
    Io {
        path: String,
        source: io::Error,
    },
    Toml(toml::de::Error),
    SchemaMismatch {
        expected: &'static str,
        observed: String,
    },
    ByteLengthMismatch {
        path: String,
        expected: u64,
        observed: u64,
    },
    Sha256Mismatch {
        path: String,
        expected: Hash256,
        observed: Hash256,
    },
    LengthOverflow {
        path: String,
    },
}

impl fmt::Display for CorpusManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::SchemaMismatch { expected, observed } => {
                write!(
                    f,
                    "expected manifest schema {expected:?}, observed {observed:?}"
                )
            }
            Self::ByteLengthMismatch {
                path,
                expected,
                observed,
            } => write!(f, "{path}: expected {expected} bytes, observed {observed}"),
            Self::Sha256Mismatch {
                path,
                expected,
                observed,
            } => write!(f, "{path}: expected sha256 {expected}, observed {observed}"),
            Self::LengthOverflow { path } => write!(f, "{path}: byte length overflow"),
        }
    }
}

impl std::error::Error for CorpusManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::SchemaMismatch { .. } => None,
            Self::ByteLengthMismatch { .. }
            | Self::Sha256Mismatch { .. }
            | Self::LengthOverflow { .. } => None,
        }
    }
}
