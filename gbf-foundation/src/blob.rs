//! Blob handles shared across contract crates.

use serde::{Deserialize, Serialize};

use crate::Hash256;

/// A handle to a content-addressed blob.
///
/// `BlobRef` carries the canonical content hash, the stored byte length, and
/// the codec chosen by the blob producer. Storage layers keep the bytes opaque;
/// consumers interpret `codec` when they materialize the blob.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BlobRef {
    #[serde(with = "hash_hex")]
    pub hash: Hash256,
    pub len: u32,
    pub codec: BlobCodec,
}

/// Encoding applied to the stored blob bytes by the producer.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobCodec {
    Raw,
    Zstd,
}

mod hash_hex {
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    use crate::Hash256;

    pub fn serialize<S>(hash: &Hash256, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hash.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Hash256, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Hash256::from_str(&value).map_err(serde::de::Error::custom)
    }
}
