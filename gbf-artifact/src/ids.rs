//! Artifact-local identifiers.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactPath(String);

impl ArtifactPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ArtifactPathError> {
        let value = value.into();
        validate_artifact_path(&value)?;
        Ok(Self(value))
    }

    pub fn join(&self, segment: &str) -> Result<Self, ArtifactPathError> {
        validate_artifact_segment(segment)?;
        Self::new(format!("{}.{}", self.0, segment))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for ArtifactPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ArtifactPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactPathError {
    Empty,
    EmptySegment,
    InvalidCharacter { index: usize, byte: u8 },
}

impl fmt::Display for ArtifactPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("artifact path must not be empty"),
            Self::EmptySegment => f.write_str("artifact path segments must not be empty"),
            Self::InvalidCharacter { index, byte } => {
                write!(
                    f,
                    "artifact path contains invalid byte 0x{byte:02x} at index {index}"
                )
            }
        }
    }
}

impl Error for ArtifactPathError {}

fn validate_artifact_path(value: &str) -> Result<(), ArtifactPathError> {
    if value.is_empty() {
        return Err(ArtifactPathError::Empty);
    }

    for segment in value.split('.') {
        validate_artifact_segment(segment)?;
    }

    for (index, byte) in value.bytes().enumerate() {
        if is_artifact_path_byte(byte) || byte == b'.' {
            continue;
        }

        return Err(ArtifactPathError::InvalidCharacter { index, byte });
    }

    Ok(())
}

fn validate_artifact_segment(segment: &str) -> Result<(), ArtifactPathError> {
    if segment.is_empty() {
        return Err(ArtifactPathError::EmptySegment);
    }

    for (index, byte) in segment.bytes().enumerate() {
        if is_artifact_path_byte(byte) {
            continue;
        }

        return Err(ArtifactPathError::InvalidCharacter { index, byte });
    }

    Ok(())
}

fn is_artifact_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_paths_validate_segmented_names() {
        let path = ArtifactPath::new("layer.0.expert_1.up-weight").unwrap();

        assert_eq!(
            path.join("scale").unwrap().as_str(),
            "layer.0.expert_1.up-weight.scale"
        );
        assert_eq!(ArtifactPath::new(""), Err(ArtifactPathError::Empty));
        assert_eq!(
            ArtifactPath::new("layer..weight"),
            Err(ArtifactPathError::EmptySegment)
        );
        assert!(matches!(
            ArtifactPath::new("layer/0"),
            Err(ArtifactPathError::InvalidCharacter { .. })
        ));
    }
}
