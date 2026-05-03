//! Named GC root sets and typed blob-reference contracts.

use std::collections::BTreeSet;
use std::fmt;

use gbf_foundation::Hash256;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PinsetName(String);

impl PinsetName {
    pub fn new(value: impl Into<String>) -> Result<Self, PinsetNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PinsetNameError::Empty);
        }
        if value.as_bytes().contains(&0) {
            return Err(PinsetNameError::ContainsNul);
        }
        if value.contains('/') || value.contains('\\') {
            return Err(PinsetNameError::ContainsPathSeparator);
        }
        if value.contains("..") {
            return Err(PinsetNameError::ParentSegment);
        }
        if value.starts_with('.') {
            return Err(PinsetNameError::LeadingDot);
        }
        Ok(Self(value))
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

impl TryFrom<String> for PinsetName {
    type Error = PinsetNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for PinsetName {
    type Error = PinsetNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for PinsetName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PinsetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PinsetName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PinsetNameError {
    Empty,
    ContainsNul,
    ContainsPathSeparator,
    ParentSegment,
    LeadingDot,
}

impl fmt::Display for PinsetNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("pinset name must not be empty"),
            Self::ContainsNul => f.write_str("pinset name must not contain NUL"),
            Self::ContainsPathSeparator => {
                f.write_str("pinset name must not contain path separators")
            }
            Self::ParentSegment => f.write_str("pinset name must not contain '..'"),
            Self::LeadingDot => f.write_str("pinset name must not start with '.'"),
        }
    }
}

impl std::error::Error for PinsetNameError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pinset {
    pub name: PinsetName,
    pub roots: BTreeSet<Hash256>,
    pub annotation: Option<String>,
}

pub trait BlobReferences {
    fn referenced_blobs(&self) -> Vec<Hash256>;
}
