//! Shared 256-bit content hash wrapper.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

const SHA256_PREFIX: &str = "sha256:";

/// A SHA-256-sized digest used for stable cross-crate identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash256([u8; 32]);

impl Hash256 {
    /// The all-zero digest, useful as an explicit sentinel in tests and fixtures.
    pub const ZERO: Self = Self([0; 32]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        let mut hex = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write as _;
            write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
        }
        hex
    }
}

/// Compute a SHA-256 digest and wrap it in the project's canonical hash type.
#[must_use]
pub fn sha256(bytes: impl AsRef<[u8]>) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes.as_ref()).into())
}

impl From<[u8; 32]> for Hash256 {
    fn from(bytes: [u8; 32]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl From<Hash256> for [u8; 32] {
    fn from(hash: Hash256) -> Self {
        hash.to_bytes()
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SHA256_PREFIX)?;
        f.write_str(&self.to_hex())?;
        Ok(())
    }
}

impl FromStr for Hash256 {
    type Err = Hash256ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix(SHA256_PREFIX)
            .ok_or(Hash256ParseError::InvalidPrefix)?;
        if hex.len() != 64 {
            return Err(Hash256ParseError::InvalidLength {
                expected: 64,
                actual: hex.len(),
            });
        }

        let mut bytes = [0_u8; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_value(pair[0]).ok_or(Hash256ParseError::InvalidHex {
                index: index * 2,
                byte: pair[0],
            })?;
            let low = hex_value(pair[1]).ok_or(Hash256ParseError::InvalidHex {
                index: index * 2 + 1,
                byte: pair[1],
            })?;
            bytes[index] = (high << 4) | low;
        }

        Ok(Self(bytes))
    }
}

impl Serialize for Hash256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hash256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Hash256ParseError {
    InvalidPrefix,
    InvalidLength { expected: usize, actual: usize },
    InvalidHex { index: usize, byte: u8 },
}

impl fmt::Display for Hash256ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrefix => write!(f, "expected sha256: prefix"),
            Self::InvalidLength { expected, actual } => {
                write!(f, "expected {expected} hex characters, got {actual}")
            }
            Self::InvalidHex { index, byte } => {
                write!(f, "invalid hex byte 0x{byte:02x} at index {index}")
            }
        }
    }
}

impl std::error::Error for Hash256ParseError {}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash256_displays_and_parses_hex() {
        let hash = Hash256::from_bytes([0xab; 32]);

        assert_eq!(
            hash.to_string(),
            "sha256:abababababababababababababababababababababababababababababababab"
        );
        assert_eq!(hash.to_string().parse::<Hash256>(), Ok(hash));
    }

    #[test]
    fn hash256_rejects_bad_hex_contracts() {
        assert_eq!(
            "abc".parse::<Hash256>(),
            Err(Hash256ParseError::InvalidPrefix)
        );
        assert_eq!(
            "sha256:abc".parse::<Hash256>(),
            Err(Hash256ParseError::InvalidLength {
                expected: 64,
                actual: 3,
            })
        );
        assert_eq!(
            "sha256:ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB"
                .parse::<Hash256>(),
            Err(Hash256ParseError::InvalidHex {
                index: 0,
                byte: b'A',
            })
        );
        assert_eq!(
            "sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
                .parse::<Hash256>(),
            Err(Hash256ParseError::InvalidHex {
                index: 0,
                byte: b'g',
            })
        );
    }

    #[test]
    fn hash256_round_trips_through_serde() {
        let hash = Hash256::from_bytes([7; 32]);
        let encoded = serde_json::to_string(&hash).expect("hash serializes");
        let decoded: Hash256 = serde_json::from_str(&encoded).expect("hash deserializes");

        assert_eq!(
            encoded,
            "\"sha256:0707070707070707070707070707070707070707070707070707070707070707\""
        );
        assert_eq!(decoded, hash);
        assert_eq!(decoded.as_bytes(), &[7; 32]);
    }

    #[test]
    fn sha256_helper_returns_prefixed_hash() {
        assert_eq!(
            sha256(b"abc").to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
