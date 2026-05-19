//! S3 TinyStories.v2 manifest pins and verification.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use gbf_artifact::{LexicalSpec_v1, TextCharSeq};
use gbf_foundation::{Hash256, sha256};
use serde::{Deserialize, Serialize};

use crate::charset_v1::{CharsetError, CharsetInputs, normalize_raw, s3_charset_v1};
use crate::corpus::{CorpusManifestError, SplitRole, read_tinystories_manifest};

pub const TINYSTORIES_V2_MANIFEST_SCHEMA: &str = "tinystories_v2_manifest.v1";
pub const TINYSTORIES_V2_MANIFEST_LOADED_EVENT: &str = "s3::tinystories_v2_manifest::loaded";
pub const TINYSTORIES_V2_MANIFEST_LOG_TARGET: &str = "gbf_data::manifest";

/// S3 TinyStories manifest pinning raw and charset_v1-normalized corpus hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TinyStoriesV2Manifest {
    #[serde(skip)]
    manifest_dir: Option<PathBuf>,
    pub schema: String,
    pub schema_version: String,
    pub corpus_id: String,
    pub dataset_version: String,
    pub source_manifest_path: String,
    pub fixture_mode: bool,
    pub fixture_boundary: String,
    pub raw_train_path: String,
    pub raw_val_path: String,
    pub raw_train_sha256: Hash256,
    pub raw_val_sha256: Hash256,
    pub raw_train_byte_count: u64,
    pub raw_val_byte_count: u64,
    pub post_hash_input: String,
    pub fixture_raw_train_path: String,
    pub fixture_raw_val_path: String,
    pub fixture_raw_train_sha256: Hash256,
    pub fixture_raw_val_sha256: Hash256,
    pub fixture_raw_train_byte_count: u64,
    pub fixture_raw_val_byte_count: u64,
    pub train_post_sha256: Hash256,
    pub val_post_sha256: Hash256,
    pub train_post_char_count: u64,
    pub val_post_char_count: u64,
    pub charset_v1_sha256: Hash256,
    pub story_separator: String,
    pub raw_example_boundary_policy: String,
    pub held_out_chapter_path: String,
    pub chapter_sha256: Hash256,
    pub chapter_char_count: u64,
    pub prompt_offsets: Vec<u64>,
    pub prompt_min_chars: u64,
    pub prompt_max_chars: u64,
    pub agreement_prompt_count: u64,
    pub deferred_real_corpus_owner: String,
}

impl TinyStoriesV2Manifest {
    pub fn from_toml_str(input: &str) -> Result<Self, TinyStoriesV2ManifestError> {
        let manifest: Self = toml::from_str(input).map_err(TinyStoriesV2ManifestError::Toml)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, TinyStoriesV2ManifestError> {
        let path = path.as_ref();
        let text =
            std::fs::read_to_string(path).map_err(|source| TinyStoriesV2ManifestError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let mut manifest = Self::from_toml_str(&text)?;
        manifest.manifest_dir = path.parent().map(Path::to_path_buf);
        emit_manifest_loaded(&manifest);
        Ok(manifest)
    }

    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(manifest_dir) = &self.manifest_dir {
            manifest_dir.join(path)
        } else {
            path.to_path_buf()
        }
    }

    fn validate(&self) -> Result<(), TinyStoriesV2ManifestError> {
        if self.schema != TINYSTORIES_V2_MANIFEST_SCHEMA {
            return Err(TinyStoriesV2ManifestError::SchemaMismatch {
                expected: TINYSTORIES_V2_MANIFEST_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        if self.prompt_offsets.len() != 8 {
            return Err(TinyStoriesV2ManifestError::PromptCount {
                expected: 8,
                observed: self.prompt_offsets.len(),
            });
        }
        if self.agreement_prompt_count != 3 {
            return Err(TinyStoriesV2ManifestError::ManifestInvariant {
                message: format!(
                    "agreement_prompt_count must be 3, observed {}",
                    self.agreement_prompt_count
                ),
            });
        }
        let expected_post_hash_input = if self.fixture_mode {
            "fixture_raw_bytes"
        } else {
            "raw_tinystories_bytes"
        };
        if self.post_hash_input != expected_post_hash_input {
            return Err(TinyStoriesV2ManifestError::ManifestInvariant {
                message: format!(
                    "post_hash_input must be {expected_post_hash_input:?} when fixture_mode is {}, observed {:?}",
                    self.fixture_mode, self.post_hash_input
                ),
            });
        }
        if self.prompt_min_chars < 64
            || self.prompt_max_chars > 128
            || self.prompt_min_chars > self.prompt_max_chars
        {
            return Err(TinyStoriesV2ManifestError::ManifestInvariant {
                message: format!(
                    "prompt bounds must satisfy 64 <= min <= max <= 128, observed {}..{}",
                    self.prompt_min_chars, self.prompt_max_chars
                ),
            });
        }
        for (index, offset) in self.prompt_offsets.iter().copied().enumerate() {
            if offset.saturating_add(self.prompt_max_chars) > self.chapter_char_count {
                return Err(TinyStoriesV2ManifestError::PromptOffsetOutOfRange {
                    index,
                    offset,
                    prompt_max_chars: self.prompt_max_chars,
                    chapter_char_count: self.chapter_char_count,
                });
            }
        }
        Ok(())
    }
}

pub fn read_tinystories_v2_manifest(
    path: impl AsRef<Path>,
) -> Result<TinyStoriesV2Manifest, TinyStoriesV2ManifestError> {
    TinyStoriesV2Manifest::from_toml_file(path)
}

/// Recomputed manifest evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyStoriesV2Verification {
    pub train_post: TextCharSeq,
    pub val_post: TextCharSeq,
    pub held_out_chapter: TextCharSeq,
}

pub fn verify_tinystories_v2_manifest(
    manifest: &TinyStoriesV2Manifest,
) -> Result<TinyStoriesV2Verification, TinyStoriesV2ManifestError> {
    verify_source_manifest_pins(manifest)?;
    let train_inputs = NormalizationBytesSpec {
        split: SplitRole::Train,
        raw_path: &manifest.raw_train_path,
        raw_hash: manifest.raw_train_sha256,
        raw_len: manifest.raw_train_byte_count,
        fixture_path: &manifest.fixture_raw_train_path,
        fixture_hash: manifest.fixture_raw_train_sha256,
        fixture_len: manifest.fixture_raw_train_byte_count,
    };
    let val_inputs = NormalizationBytesSpec {
        split: SplitRole::Validation,
        raw_path: &manifest.raw_val_path,
        raw_hash: manifest.raw_val_sha256,
        raw_len: manifest.raw_val_byte_count,
        fixture_path: &manifest.fixture_raw_val_path,
        fixture_hash: manifest.fixture_raw_val_sha256,
        fixture_len: manifest.fixture_raw_val_byte_count,
    };
    let raw_train = read_normalization_bytes(manifest, train_inputs)?;
    let raw_val = read_normalization_bytes(manifest, val_inputs)?;
    let chapter = read_verified_bytes(
        manifest,
        &manifest.held_out_chapter_path,
        "chapter_sha256",
        manifest.chapter_sha256,
        None,
    )?;

    let product = s3_charset_v1(CharsetInputs {
        raw_train_examples: vec![raw_train],
        raw_val_examples: vec![raw_val],
        spec: LexicalSpec_v1::pinned(),
    })
    .map_err(TinyStoriesV2ManifestError::Charset)?;

    check_hash_field(
        "train_post_sha256",
        manifest.train_post_sha256,
        product.train_post_sha256,
    )?;
    check_hash_field(
        "val_post_sha256",
        manifest.val_post_sha256,
        product.val_post_sha256,
    )?;
    check_hash_field(
        "charset_v1_sha256",
        manifest.charset_v1_sha256,
        product.charset_v1_sha256,
    )?;
    check_count_field(
        "train_post_char_count",
        manifest.train_post_char_count,
        product.train_post.len() as u64,
    )?;
    check_count_field(
        "val_post_char_count",
        manifest.val_post_char_count,
        product.val_post.len() as u64,
    )?;

    let chapter_stats = normalize_raw(&chapter).map_err(TinyStoriesV2ManifestError::Charset)?;
    if chapter_stats.dropped {
        return Err(TinyStoriesV2ManifestError::ManifestInvariant {
            message: "held-out chapter is dropped by charset_v1 normalization".to_owned(),
        });
    }
    check_count_field(
        "chapter_char_count",
        manifest.chapter_char_count,
        chapter_stats.tokens.len() as u64,
    )?;

    Ok(TinyStoriesV2Verification {
        train_post: product.train_post,
        val_post: product.val_post,
        held_out_chapter: chapter_stats.tokens,
    })
}

fn verify_source_manifest_pins(
    manifest: &TinyStoriesV2Manifest,
) -> Result<(), TinyStoriesV2ManifestError> {
    let source_path = manifest.resolve_path(&manifest.source_manifest_path);
    let source = read_tinystories_manifest(&source_path).map_err(|source| {
        TinyStoriesV2ManifestError::SourceManifest {
            path: source_path.display().to_string(),
            source,
        }
    })?;
    check_hash_field(
        "raw_train_sha256",
        manifest.raw_train_sha256,
        source.train_sha256,
    )?;
    check_hash_field("raw_val_sha256", manifest.raw_val_sha256, source.val_sha256)?;
    check_count_field(
        "raw_train_byte_count",
        manifest.raw_train_byte_count,
        source.file(SplitRole::Train).byte_length,
    )?;
    check_count_field(
        "raw_val_byte_count",
        manifest.raw_val_byte_count,
        source.file(SplitRole::Validation).byte_length,
    )?;
    Ok(())
}

struct NormalizationBytesSpec<'a> {
    split: SplitRole,
    raw_path: &'a str,
    raw_hash: Hash256,
    raw_len: u64,
    fixture_path: &'a str,
    fixture_hash: Hash256,
    fixture_len: u64,
}

fn read_normalization_bytes(
    manifest: &TinyStoriesV2Manifest,
    spec: NormalizationBytesSpec<'_>,
) -> Result<Vec<u8>, TinyStoriesV2ManifestError> {
    let (path, hash_field, hash, len_field, len) = if manifest.fixture_mode {
        match spec.split {
            SplitRole::Train => (
                spec.fixture_path,
                "fixture_raw_train_sha256",
                spec.fixture_hash,
                "fixture_raw_train_byte_count",
                spec.fixture_len,
            ),
            SplitRole::Validation => (
                spec.fixture_path,
                "fixture_raw_val_sha256",
                spec.fixture_hash,
                "fixture_raw_val_byte_count",
                spec.fixture_len,
            ),
        }
    } else {
        match spec.split {
            SplitRole::Train => (
                spec.raw_path,
                "raw_train_sha256",
                spec.raw_hash,
                "raw_train_byte_count",
                spec.raw_len,
            ),
            SplitRole::Validation => (
                spec.raw_path,
                "raw_val_sha256",
                spec.raw_hash,
                "raw_val_byte_count",
                spec.raw_len,
            ),
        }
    };
    read_verified_bytes(manifest, path, hash_field, hash, Some((len_field, len)))
}

fn read_verified_bytes(
    manifest: &TinyStoriesV2Manifest,
    manifest_path: &str,
    hash_field: &'static str,
    expected_hash: Hash256,
    expected_len: Option<(&'static str, u64)>,
) -> Result<Vec<u8>, TinyStoriesV2ManifestError> {
    let path = manifest.resolve_path(manifest_path);
    let bytes = std::fs::read(&path).map_err(|source| TinyStoriesV2ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    if let Some((len_field, expected)) = expected_len {
        let observed =
            u64::try_from(bytes.len()).map_err(|_| TinyStoriesV2ManifestError::LengthOverflow {
                path: path.display().to_string(),
            })?;
        if observed != expected {
            return Err(TinyStoriesV2ManifestError::ByteLengthMismatch {
                field: len_field,
                path: path.display().to_string(),
                expected,
                observed,
            });
        }
    }
    let observed_hash = sha256(&bytes);
    if observed_hash != expected_hash {
        return Err(TinyStoriesV2ManifestError::HashMismatch {
            field: hash_field,
            expected: expected_hash,
            observed: observed_hash,
        });
    }
    Ok(bytes)
}

fn check_hash_field(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), TinyStoriesV2ManifestError> {
    if expected == observed {
        Ok(())
    } else {
        Err(TinyStoriesV2ManifestError::HashMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn check_count_field(
    field: &'static str,
    expected: u64,
    observed: u64,
) -> Result<(), TinyStoriesV2ManifestError> {
    if expected == observed {
        Ok(())
    } else {
        Err(TinyStoriesV2ManifestError::CountMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn emit_manifest_loaded(manifest: &TinyStoriesV2Manifest) {
    tracing::info!(
        target: TINYSTORIES_V2_MANIFEST_LOG_TARGET,
        event_name = TINYSTORIES_V2_MANIFEST_LOADED_EVENT,
        raw_train_sha256 = %manifest.raw_train_sha256,
        raw_val_sha256 = %manifest.raw_val_sha256,
        train_post_sha256 = %manifest.train_post_sha256,
        val_post_sha256 = %manifest.val_post_sha256,
        charset_v1_sha256 = %manifest.charset_v1_sha256,
        chapter_sha256 = %manifest.chapter_sha256,
        chapter_char_count = manifest.chapter_char_count,
        prompt_count = manifest.prompt_offsets.len() as u64,
    );
}

#[derive(Debug)]
pub enum TinyStoriesV2ManifestError {
    Io {
        path: String,
        source: std::io::Error,
    },
    Toml(toml::de::Error),
    SourceManifest {
        path: String,
        source: CorpusManifestError,
    },
    SchemaMismatch {
        expected: &'static str,
        observed: String,
    },
    ByteLengthMismatch {
        field: &'static str,
        path: String,
        expected: u64,
        observed: u64,
    },
    HashMismatch {
        field: &'static str,
        expected: Hash256,
        observed: Hash256,
    },
    CountMismatch {
        field: &'static str,
        expected: u64,
        observed: u64,
    },
    LengthOverflow {
        path: String,
    },
    PromptCount {
        expected: usize,
        observed: usize,
    },
    PromptOffsetOutOfRange {
        index: usize,
        offset: u64,
        prompt_max_chars: u64,
        chapter_char_count: u64,
    },
    ManifestInvariant {
        message: String,
    },
    Charset(CharsetError),
}

impl fmt::Display for TinyStoriesV2ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::SourceManifest { path, source } => {
                write!(f, "source manifest {path}: {source}")
            }
            Self::SchemaMismatch { expected, observed } => {
                write!(
                    f,
                    "expected manifest schema {expected:?}, observed {observed:?}"
                )
            }
            Self::ByteLengthMismatch {
                field,
                path,
                expected,
                observed,
            } => write!(
                f,
                "{field} for {path}: expected {expected} bytes, observed {observed}"
            ),
            Self::HashMismatch {
                field,
                expected,
                observed,
            } => write!(f, "{field}: expected {expected}, observed {observed}"),
            Self::CountMismatch {
                field,
                expected,
                observed,
            } => write!(f, "{field}: expected {expected}, observed {observed}"),
            Self::LengthOverflow { path } => write!(f, "{path}: byte length overflow"),
            Self::PromptCount { expected, observed } => {
                write!(f, "expected {expected} prompt offsets, observed {observed}")
            }
            Self::PromptOffsetOutOfRange {
                index,
                offset,
                prompt_max_chars,
                chapter_char_count,
            } => write!(
                f,
                "prompt offset {index} at {offset} plus max length {prompt_max_chars} exceeds chapter_char_count {chapter_char_count}"
            ),
            Self::ManifestInvariant { message } => f.write_str(message),
            Self::Charset(error) => write!(f, "{error}"),
        }
    }
}

impl Error for TinyStoriesV2ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::SourceManifest { source, .. } => Some(source),
            Self::Charset(error) => Some(error),
            Self::SchemaMismatch { .. }
            | Self::ByteLengthMismatch { .. }
            | Self::HashMismatch { .. }
            | Self::CountMismatch { .. }
            | Self::LengthOverflow { .. }
            | Self::PromptCount { .. }
            | Self::PromptOffsetOutOfRange { .. }
            | Self::ManifestInvariant { .. } => None,
        }
    }
}
