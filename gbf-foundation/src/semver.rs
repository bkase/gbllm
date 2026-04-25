//! Minimal semantic version wrapper.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A simple `major.minor.patch` semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for SemVer {
    type Err = SemVerParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');
        let major = parse_component(parts.next(), "major")?;
        let minor = parse_component(parts.next(), "minor")?;
        let patch = parse_component(parts.next(), "patch")?;

        if parts.next().is_some() {
            return Err(SemVerParseError::InvalidFormat);
        }

        Ok(Self::new(major, minor, patch))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemVerParseError {
    InvalidFormat,
    InvalidNumber { component: &'static str },
}

impl fmt::Display for SemVerParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => {
                f.write_str("expected semantic version in major.minor.patch form")
            }
            Self::InvalidNumber { component } => {
                write!(f, "invalid {component} semantic version component")
            }
        }
    }
}

impl std::error::Error for SemVerParseError {}

fn parse_component(component: Option<&str>, name: &'static str) -> Result<u64, SemVerParseError> {
    component
        .ok_or(SemVerParseError::InvalidFormat)?
        .parse()
        .map_err(|_| SemVerParseError::InvalidNumber { component: name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_displays_parses_and_orders() {
        let version = SemVer::new(1, 2, 3);

        assert_eq!(version.to_string(), "1.2.3");
        assert_eq!("1.2.3".parse::<SemVer>(), Ok(version));
        assert!(SemVer::new(1, 2, 4) > version);
    }

    #[test]
    fn semver_rejects_invalid_contracts() {
        assert_eq!(
            "1.2".parse::<SemVer>(),
            Err(SemVerParseError::InvalidFormat)
        );
        assert_eq!(
            "1.two.3".parse::<SemVer>(),
            Err(SemVerParseError::InvalidNumber { component: "minor" })
        );
    }

    #[test]
    fn semver_round_trips_through_serde() {
        let encoded = serde_json::to_string(&SemVer::new(2, 0, 1)).expect("semver serializes");
        let decoded: SemVer = serde_json::from_str(&encoded).expect("semver deserializes");

        assert_eq!(decoded, SemVer::new(2, 0, 1));
    }
}
