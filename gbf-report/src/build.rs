//! Build report package helpers.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use gbf_foundation::Hash256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReportPackageEntry {
    pub relative_path: String,
    pub report_self_hash: Hash256,
    pub canonical_bytes: Vec<u8>,
}

impl BuildReportPackageEntry {
    pub fn new(
        relative_path: impl Into<String>,
        report_self_hash: Hash256,
        canonical_bytes: Vec<u8>,
    ) -> Result<Self, BuildReportPackageError> {
        let relative_path = relative_path.into();
        validate_relative_path(&relative_path)?;
        Ok(Self {
            relative_path,
            report_self_hash,
            canonical_bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildReportPackage {
    entries: Vec<BuildReportPackageEntry>,
}

impl BuildReportPackage {
    pub fn from_entries(
        entries: impl IntoIterator<Item = BuildReportPackageEntry>,
    ) -> Result<Self, BuildReportPackageError> {
        let mut by_path = BTreeMap::new();
        for entry in entries {
            validate_relative_path(&entry.relative_path)?;
            if by_path.contains_key(&entry.relative_path) {
                return Err(BuildReportPackageError::DuplicateEntryPath {
                    relative_path: entry.relative_path,
                });
            }
            by_path.insert(entry.relative_path.clone(), entry);
        }
        Ok(Self {
            entries: by_path.into_values().collect(),
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[BuildReportPackageEntry] {
        &self.entries
    }

    #[must_use]
    pub fn get(&self, relative_path: &str) -> Option<&BuildReportPackageEntry> {
        self.entries
            .binary_search_by(|entry| entry.relative_path.as_str().cmp(relative_path))
            .ok()
            .map(|index| &self.entries[index])
    }

    pub fn write_to_dir(
        &self,
        output_dir: impl AsRef<Path>,
    ) -> Result<(), BuildReportPackageError> {
        let output_dir = output_dir.as_ref();
        fs::create_dir_all(output_dir).map_err(|source| BuildReportPackageError::Io {
            path: output_dir.to_path_buf(),
            source,
        })?;
        for entry in &self.entries {
            let path = output_dir.join(relative_path_buf(&entry.relative_path));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| BuildReportPackageError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::write(&path, &entry.canonical_bytes)
                .map_err(|source| BuildReportPackageError::Io { path, source })?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum BuildReportPackageError {
    InvalidEntryPath {
        relative_path: String,
        reason: &'static str,
    },
    DuplicateEntryPath {
        relative_path: String,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for BuildReportPackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEntryPath {
                relative_path,
                reason,
            } => write!(
                f,
                "invalid build report package path {relative_path:?}: {reason}"
            ),
            Self::DuplicateEntryPath { relative_path } => {
                write!(f, "duplicate build report package path {relative_path:?}")
            }
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to write build report package path {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for BuildReportPackageError {}

fn validate_relative_path(relative_path: &str) -> Result<(), BuildReportPackageError> {
    if relative_path.is_empty() {
        return Err(invalid_path(relative_path, "path must not be empty"));
    }
    if relative_path.starts_with('/') {
        return Err(invalid_path(relative_path, "path must be package-relative"));
    }
    if relative_path.contains('\\') {
        return Err(invalid_path(
            relative_path,
            "path must use forward slash separators",
        ));
    }
    for part in relative_path.split('/') {
        if part.is_empty() {
            return Err(invalid_path(
                relative_path,
                "path must not contain empty segments",
            ));
        }
        if part == "." || part == ".." {
            return Err(invalid_path(
                relative_path,
                "path must not contain current or parent directory segments",
            ));
        }
    }
    Ok(())
}

fn invalid_path(relative_path: &str, reason: &'static str) -> BuildReportPackageError {
    BuildReportPackageError::InvalidEntryPath {
        relative_path: relative_path.to_owned(),
        reason,
    }
}

fn relative_path_buf(relative_path: &str) -> PathBuf {
    relative_path.split('/').collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn build_report_package_rejects_duplicate_and_unsafe_paths() {
        let first = entry("cache_status.json", 0x10, b"first");
        let duplicate = entry("cache_status.json", 0x11, b"second");
        assert!(matches!(
            BuildReportPackage::from_entries([first, duplicate]),
            Err(BuildReportPackageError::DuplicateEntryPath { relative_path })
                if relative_path == "cache_status.json"
        ));

        for path in [
            "",
            "/cache_status.json",
            "reports//x.json",
            "../x.json",
            "x\\y.json",
        ] {
            assert!(matches!(
                BuildReportPackageEntry::new(path, hash(0x20), b"bad".to_vec()),
                Err(BuildReportPackageError::InvalidEntryPath { .. })
            ));
        }
    }

    #[test]
    fn build_report_package_writes_entries_in_canonical_path_order() {
        let package = BuildReportPackage::from_entries([
            entry("certs/reachability.cert.json", 0x31, b"cert"),
            entry("cache_status.json", 0x30, b"cache"),
        ])
        .expect("package");
        assert_eq!(package.entries()[0].relative_path, "cache_status.json");
        assert_eq!(
            package.entries()[1].relative_path,
            "certs/reachability.cert.json"
        );

        let dir = temp_dir();
        package.write_to_dir(&dir).expect("package writes");
        assert_eq!(
            fs::read(dir.join("cache_status.json")).expect("cache_status reads"),
            b"cache"
        );
        assert_eq!(
            fs::read(dir.join("certs").join("reachability.cert.json")).expect("cert reads"),
            b"cert"
        );
        fs::remove_dir_all(dir).expect("cleanup temp package dir");
    }

    fn entry(path: &str, hash_byte: u8, bytes: &'static [u8]) -> BuildReportPackageEntry {
        BuildReportPackageEntry::new(path, hash(hash_byte), bytes.to_vec()).expect("entry")
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "gbf-report-build-package-{}-{nanos}-{id}",
            std::process::id()
        ))
    }
}
