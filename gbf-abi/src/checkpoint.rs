//! Semantic checkpoint identifiers and build-local compact ids.

use core::fmt;

use serde::{Deserialize, Serialize};

#[cfg(feature = "host")]
use crate::version::{AbiVersion, BuildIdentityBlock};

/// Build-local compact checkpoint id used in byte-constrained runtime state.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CompactCheckpointId(pub u16);

impl CompactCheckpointId {
    /// Sentinel meaning "no checkpoint reached yet".
    pub const NONE: Self = Self(0);
}

/// Three checkpoint strata shared by the oracle stack and runtime reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SemanticStratum {
    Denotation,
    Artifact,
    Operational,
}

impl SemanticStratum {
    pub const ALL: [Self; 3] = [Self::Denotation, Self::Artifact, Self::Operational];
}

/// Semantic checkpoint id parser/constructor errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointIdError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidChar { byte: u8, position: usize },
    LeadingDot,
    TrailingDot,
    DoubleDot { position: usize },
}

impl fmt::Display for CheckpointIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("semantic checkpoint id is empty"),
            Self::TooLong { len, max } => {
                write!(f, "semantic checkpoint id length {len} exceeds max {max}")
            }
            Self::InvalidChar { byte, position } => {
                write!(
                    f,
                    "semantic checkpoint id byte 0x{byte:02x} at position {position} is invalid"
                )
            }
            Self::LeadingDot => f.write_str("semantic checkpoint id has a leading dot"),
            Self::TrailingDot => f.write_str("semantic checkpoint id has a trailing dot"),
            Self::DoubleDot { position } => {
                write!(f, "semantic checkpoint id has a double dot at {position}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CheckpointIdError {}

#[cfg(feature = "alloc")]
mod semantic_id {
    use alloc::borrow::Cow;
    use alloc::string::String;
    use core::fmt;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{CheckpointIdError, validate_semantic_checkpoint_id};

    /// Durable dotted semantic checkpoint id.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SemanticCheckpointId(Cow<'static, str>);

    impl SemanticCheckpointId {
        pub fn from_static(s: &'static str) -> Result<Self, CheckpointIdError> {
            validate_semantic_checkpoint_id(s)?;
            Ok(Self(Cow::Borrowed(s)))
        }

        pub fn from_owned(s: String) -> Result<Self, CheckpointIdError> {
            validate_semantic_checkpoint_id(&s)?;
            Ok(Self(Cow::Owned(s)))
        }

        #[must_use]
        pub fn as_str(&self) -> &str {
            self.0.as_ref()
        }
    }

    impl fmt::Display for SemanticCheckpointId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.as_str())
        }
    }

    impl Serialize for SemanticCheckpointId {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for SemanticCheckpointId {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = String::deserialize(deserializer)?;
            Self::from_owned(value).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(feature = "alloc")]
pub use semantic_id::SemanticCheckpointId;

#[cfg(feature = "alloc")]
fn validate_semantic_checkpoint_id(s: &str) -> Result<(), CheckpointIdError> {
    const MAX_LEN: usize = 128;

    if s.is_empty() {
        return Err(CheckpointIdError::Empty);
    }
    if s.len() > MAX_LEN {
        return Err(CheckpointIdError::TooLong {
            len: s.len(),
            max: MAX_LEN,
        });
    }
    if s.as_bytes()[0] == b'.' {
        return Err(CheckpointIdError::LeadingDot);
    }
    if s.as_bytes()[s.len() - 1] == b'.' {
        return Err(CheckpointIdError::TrailingDot);
    }

    let mut prev_dot = false;
    for (position, byte) in s.bytes().enumerate() {
        match byte {
            b'.' if prev_dot => return Err(CheckpointIdError::DoubleDot { position }),
            b'.' => prev_dot = true,
            b'a'..=b'z' | b'0'..=b'9' | b'_' => prev_dot = false,
            _ => return Err(CheckpointIdError::InvalidChar { byte, position }),
        }
    }

    Ok(())
}

/// Host schema mapping durable semantic ids to compact build-local ids.
#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticCheckpointSchema {
    pub schema_version: u16,
    pub abi_version: AbiVersion,
    pub build_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub checkpoints: alloc::vec::Vec<CheckpointEntry>,
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEntry {
    pub semantic: SemanticCheckpointId,
    pub compact: CompactCheckpointId,
    pub stratum: SemanticStratum,
    pub source_op: Option<alloc::borrow::Cow<'static, str>>,
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaValidationError {
    DuplicateCompact {
        compact: CompactCheckpointId,
    },
    DuplicateSemantic {
        semantic: SemanticCheckpointId,
    },
    ReservedCompactZero,
    InvalidSemanticId {
        semantic: alloc::string::String,
        error: CheckpointIdError,
    },
    BuildHashMismatch {
        expected: [u8; 32],
        observed: [u8; 32],
    },
    CompileRequestHashMismatch {
        expected: [u8; 32],
        observed: [u8; 32],
    },
    AbiVersionMismatch {
        expected: AbiVersion,
        observed: AbiVersion,
    },
    SchemaVersionMismatch {
        expected: u16,
        observed: u16,
    },
}

#[cfg(feature = "host")]
impl fmt::Display for SchemaValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCompact { compact } => {
                write!(f, "duplicate compact checkpoint id {}", compact.0)
            }
            Self::DuplicateSemantic { semantic } => {
                write!(f, "duplicate semantic checkpoint id {semantic}")
            }
            Self::ReservedCompactZero => f.write_str("compact checkpoint id 0 is reserved"),
            Self::InvalidSemanticId { semantic, error } => {
                write!(f, "invalid semantic checkpoint id {semantic}: {error}")
            }
            Self::BuildHashMismatch { .. } => {
                f.write_str("semantic checkpoint schema build hash mismatch")
            }
            Self::CompileRequestHashMismatch { .. } => {
                f.write_str("semantic checkpoint schema compile request hash mismatch")
            }
            Self::AbiVersionMismatch { expected, observed } => write!(
                f,
                "semantic checkpoint schema ABI mismatch: expected {expected}, observed {observed}"
            ),
            Self::SchemaVersionMismatch { expected, observed } => write!(
                f,
                "semantic checkpoint schema version mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

#[cfg(all(feature = "host", feature = "std"))]
impl std::error::Error for SchemaValidationError {}

#[cfg(feature = "host")]
impl SemanticCheckpointSchema {
    pub fn validate(&self) -> Result<(), SchemaValidationError> {
        if self.schema_version == 0 {
            return Err(SchemaValidationError::SchemaVersionMismatch {
                expected: 1,
                observed: self.schema_version,
            });
        }

        let mut compact_ids = alloc::collections::BTreeSet::new();
        let mut semantic_ids = alloc::collections::BTreeSet::new();

        for entry in &self.checkpoints {
            if entry.compact == CompactCheckpointId::NONE {
                return Err(SchemaValidationError::ReservedCompactZero);
            }

            if !compact_ids.insert(entry.compact) {
                return Err(SchemaValidationError::DuplicateCompact {
                    compact: entry.compact,
                });
            }

            if !semantic_ids.insert(&entry.semantic) {
                return Err(SchemaValidationError::DuplicateSemantic {
                    semantic: entry.semantic.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn validate_against_identity(
        &self,
        identity: &BuildIdentityBlock,
    ) -> Result<(), SchemaValidationError> {
        self.validate()?;

        if self.abi_version != identity.abi {
            return Err(SchemaValidationError::AbiVersionMismatch {
                expected: identity.abi,
                observed: self.abi_version,
            });
        }
        if self.schema_version != identity.semantic_schema_version {
            return Err(SchemaValidationError::SchemaVersionMismatch {
                expected: identity.semantic_schema_version,
                observed: self.schema_version,
            });
        }
        if self.build_hash != identity.build_hash {
            return Err(SchemaValidationError::BuildHashMismatch {
                expected: identity.build_hash,
                observed: self.build_hash,
            });
        }
        if self.compile_request_hash != identity.compile_request_hash {
            return Err(SchemaValidationError::CompileRequestHashMismatch {
                expected: identity.compile_request_hash,
                observed: self.compile_request_hash,
            });
        }

        Ok(())
    }

    #[must_use]
    pub fn resolve_compact(&self, id: CompactCheckpointId) -> Option<&SemanticCheckpointId> {
        self.checkpoints
            .iter()
            .find(|entry| entry.compact == id)
            .map(|entry| &entry.semantic)
    }

    #[must_use]
    pub fn resolve_semantic(&self, id: &SemanticCheckpointId) -> Option<CompactCheckpointId> {
        self.checkpoints
            .iter()
            .find(|entry| &entry.semantic == id)
            .map(|entry| entry.compact)
    }

    #[must_use]
    pub fn group_by_stratum(
        &self,
    ) -> alloc::collections::BTreeMap<SemanticStratum, alloc::vec::Vec<&CheckpointEntry>> {
        let mut grouped = alloc::collections::BTreeMap::new();
        for entry in &self.checkpoints {
            grouped
                .entry(entry.stratum)
                .or_insert_with(alloc::vec::Vec::new)
                .push(entry);
        }
        grouped
    }
}

#[cfg(feature = "alloc")]
pub trait CheckpointResolver {
    fn resolve(&self, semantic: &SemanticCheckpointId) -> Option<CompactCheckpointId>;
    fn resolve_back(&self, compact: CompactCheckpointId) -> Option<&SemanticCheckpointId>;
    fn stratum(&self, compact: CompactCheckpointId) -> Option<SemanticStratum>;
}

#[cfg(feature = "host")]
impl CheckpointResolver for SemanticCheckpointSchema {
    /// Resolve by linearly scanning the schema entries.
    ///
    /// Hot-path consumers should build their own indexed resolver from the
    /// validated schema rather than calling this implementation per trace event.
    fn resolve(&self, semantic: &SemanticCheckpointId) -> Option<CompactCheckpointId> {
        self.resolve_semantic(semantic)
    }

    fn resolve_back(&self, compact: CompactCheckpointId) -> Option<&SemanticCheckpointId> {
        self.resolve_compact(compact)
    }

    fn stratum(&self, compact: CompactCheckpointId) -> Option<SemanticStratum> {
        self.checkpoints
            .iter()
            .find(|entry| entry.compact == compact)
            .map(|entry| entry.stratum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "host")]
    fn checkpoint_schema() -> SemanticCheckpointSchema {
        SemanticCheckpointSchema {
            schema_version: 1,
            abi_version: crate::version::CURRENT_ABI,
            build_hash: [1; 32],
            compile_request_hash: [2; 32],
            checkpoints: alloc::vec![
                CheckpointEntry {
                    semantic: SemanticCheckpointId::from_static("layer.3.router.post_top1")
                        .expect("valid id"),
                    compact: CompactCheckpointId(1),
                    stratum: SemanticStratum::Denotation,
                    source_op: Some(alloc::borrow::Cow::Borrowed("router")),
                },
                CheckpointEntry {
                    semantic: SemanticCheckpointId::from_static("layer.3.ffn.out")
                        .expect("valid id"),
                    compact: CompactCheckpointId(2),
                    stratum: SemanticStratum::Operational,
                    source_op: None,
                },
            ],
        }
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn semantic_id_validation_basic() {
        let id = SemanticCheckpointId::from_static("layer.3.router.post_top1")
            .expect("valid semantic checkpoint id");

        assert_eq!(id.as_str(), "layer.3.router.post_top1");
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn semantic_id_rejects_uppercase() {
        assert!(matches!(
            SemanticCheckpointId::from_static("Layer.3"),
            Err(CheckpointIdError::InvalidChar { .. })
        ));
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn semantic_id_rejects_double_dot() {
        assert_eq!(
            SemanticCheckpointId::from_static("layer..router"),
            Err(CheckpointIdError::DoubleDot { position: 6 })
        );
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn semantic_id_rejects_leading_dot() {
        assert_eq!(
            SemanticCheckpointId::from_static(".layer"),
            Err(CheckpointIdError::LeadingDot)
        );
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn semantic_id_deserialize_rejects_invalid() {
        let decoded = serde_json::from_str::<SemanticCheckpointId>("\"Layer.3\"");

        assert!(decoded.is_err());
    }

    #[test]
    fn compact_none_sentinel() {
        assert_eq!(CompactCheckpointId::NONE.0, 0);
    }

    #[test]
    fn stratum_exhaustive() {
        assert_eq!(SemanticStratum::ALL.len(), 3);
        assert!(SemanticStratum::ALL.contains(&SemanticStratum::Denotation));
        assert!(SemanticStratum::ALL.contains(&SemanticStratum::Artifact));
        assert!(SemanticStratum::ALL.contains(&SemanticStratum::Operational));
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_validates_unique_compact() {
        let mut schema = checkpoint_schema();
        schema.checkpoints[1].compact = schema.checkpoints[0].compact;

        assert!(matches!(
            schema.validate(),
            Err(SchemaValidationError::DuplicateCompact {
                compact: CompactCheckpointId(1)
            })
        ));
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_validates_unique_semantic() {
        let mut schema = checkpoint_schema();
        schema.checkpoints[1].semantic = schema.checkpoints[0].semantic.clone();

        assert!(matches!(
            schema.validate(),
            Err(SchemaValidationError::DuplicateSemantic { .. })
        ));
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_rejects_compact_zero() {
        let mut schema = checkpoint_schema();
        schema.checkpoints[0].compact = CompactCheckpointId::NONE;

        assert_eq!(
            schema.validate(),
            Err(SchemaValidationError::ReservedCompactZero)
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_rejects_zero_schema_version() {
        let mut schema = checkpoint_schema();
        schema.schema_version = 0;

        assert_eq!(
            schema.validate(),
            Err(SchemaValidationError::SchemaVersionMismatch {
                expected: 1,
                observed: 0
            })
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_resolve_round_trip() {
        let schema = checkpoint_schema();
        schema.validate().expect("schema validates");

        for entry in &schema.checkpoints {
            assert_eq!(schema.resolve_compact(entry.compact), Some(&entry.semantic));
            assert_eq!(
                schema.resolve_semantic(&entry.semantic),
                Some(entry.compact)
            );
        }
    }

    #[test]
    #[cfg(feature = "host")]
    fn schema_validate_against_identity_rejects_schema_version_mismatch() {
        let schema = checkpoint_schema();
        let identity = BuildIdentityBlock::new(crate::version::BuildIdentityArgs {
            abi: schema.abi_version,
            build_hash: schema.build_hash,
            artifact_core_hash: [3; 32],
            runtime_nucleus_hash: [4; 32],
            compile_request_hash: schema.compile_request_hash,
            timestamp_unix: 0,
            continuation_tail_bytes: 0,
            semantic_schema_version: schema.schema_version + 1,
        });

        assert_eq!(
            schema.validate_against_identity(&identity),
            Err(SchemaValidationError::SchemaVersionMismatch {
                expected: schema.schema_version + 1,
                observed: schema.schema_version
            })
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn serde_round_trip() {
        let schema = checkpoint_schema();
        let encoded = serde_json::to_string(&schema).expect("schema serializes");
        let decoded: SemanticCheckpointSchema =
            serde_json::from_str(&encoded).expect("schema deserializes");

        assert_eq!(decoded, schema);
    }

    #[test]
    #[cfg(feature = "host")]
    fn group_by_stratum_partitions() {
        let schema = checkpoint_schema();
        let grouped = schema.group_by_stratum();

        assert_eq!(
            grouped
                .get(&SemanticStratum::Denotation)
                .expect("denotation group")
                .len(),
            1
        );
        assert_eq!(
            grouped
                .get(&SemanticStratum::Operational)
                .expect("operational group")
                .len(),
            1
        );
        assert!(!grouped.contains_key(&SemanticStratum::Artifact));
    }
}
