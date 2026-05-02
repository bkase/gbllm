//! ABI versioning and ROM-resident build identity.

use core::fmt;
#[cfg(test)]
use core::mem::{align_of, size_of};

use memoffset::offset_of;
use serde::{Deserialize, Serialize};

/// Current gbf-abi wire version.
pub const CURRENT_ABI: AbiVersion = AbiVersion {
    major: 0,
    minor: 1,
    patch: 0,
};

/// Three-byte ABI semver embedded in ROM-resident structures.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AbiVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl AbiVersion {
    #[must_use]
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.major == 0 && self.minor == 0 && self.patch == 0
    }

    #[cfg(feature = "host")]
    pub fn from_semver(v: gbf_foundation::SemVer) -> Result<Self, AbiVersionError> {
        let out_of_range = || AbiVersionError::SemVerOutOfRange {
            major: v.major,
            minor: v.minor,
            patch: v.patch,
        };
        let major = u8::try_from(v.major).map_err(|_| out_of_range())?;
        let minor = u8::try_from(v.minor).map_err(|_| out_of_range())?;
        let patch = u8::try_from(v.patch).map_err(|_| out_of_range())?;

        Ok(Self::new(major, minor, patch))
    }

    #[cfg(feature = "host")]
    #[must_use]
    pub const fn to_semver(self) -> gbf_foundation::SemVer {
        gbf_foundation::SemVer::new(self.major as u64, self.minor as u64, self.patch as u64)
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiVersionError {
    Zero,
    Unsupported {
        observed: AbiVersion,
        current: AbiVersion,
    },
    SemVerOutOfRange {
        major: u64,
        minor: u64,
        patch: u64,
    },
}

impl fmt::Display for AbiVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => f.write_str("ABI version must not be zero"),
            Self::Unsupported { observed, current } => {
                write!(
                    f,
                    "unsupported ABI version {observed}; current ABI is {current}"
                )
            }
            Self::SemVerOutOfRange {
                major,
                minor,
                patch,
            } => write!(
                f,
                "semantic version {major}.{minor}.{patch} does not fit in three u8 ABI fields"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AbiVersionError {}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityEnvelope {
    pub current: AbiVersion,
    pub backward_compatible_with: alloc::vec::Vec<AbiVersion>,
    pub forward_handshake: alloc::vec::Vec<AbiVersion>,
}

#[cfg(feature = "host")]
impl CompatibilityEnvelope {
    #[must_use]
    pub fn current_only(current: AbiVersion) -> Self {
        Self {
            current,
            backward_compatible_with: alloc::vec::Vec::new(),
            forward_handshake: alloc::vec::Vec::new(),
        }
    }

    #[must_use]
    pub fn accepts(&self, peer: AbiVersion) -> bool {
        peer == self.current
            || self.backward_compatible_with.contains(&peer)
            || self.forward_handshake.contains(&peer)
    }

    pub fn validate(&self) -> Result<(), CompatibilityError> {
        use alloc::collections::BTreeSet;

        if self.current.is_zero() {
            return Err(CompatibilityError::ZeroVersion {
                offender: self.current,
            });
        }

        let mut backward_seen = BTreeSet::new();
        for version in &self.backward_compatible_with {
            if version.is_zero() {
                return Err(CompatibilityError::ZeroVersion { offender: *version });
            }
            if *version == self.current {
                return Err(CompatibilityError::DuplicatesCurrent {
                    duplicate: *version,
                });
            }
            if *version > self.current {
                return Err(CompatibilityError::BackwardLargerThanCurrent { offender: *version });
            }
            if !backward_seen.insert(*version) {
                return Err(CompatibilityError::DuplicateInBackward { offender: *version });
            }
        }

        let mut forward_seen = BTreeSet::new();
        for version in &self.forward_handshake {
            if version.is_zero() {
                return Err(CompatibilityError::ZeroVersion { offender: *version });
            }
            if *version == self.current {
                return Err(CompatibilityError::DuplicatesCurrent {
                    duplicate: *version,
                });
            }
            if *version < self.current {
                return Err(CompatibilityError::ForwardSmallerThanCurrent { offender: *version });
            }
            if !forward_seen.insert(*version) {
                return Err(CompatibilityError::DuplicateInForward { offender: *version });
            }
        }

        Ok(())
    }
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityError {
    ZeroVersion { offender: AbiVersion },
    DuplicatesCurrent { duplicate: AbiVersion },
    BackwardLargerThanCurrent { offender: AbiVersion },
    ForwardSmallerThanCurrent { offender: AbiVersion },
    DuplicateInBackward { offender: AbiVersion },
    DuplicateInForward { offender: AbiVersion },
}

#[cfg(feature = "host")]
impl fmt::Display for CompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroVersion { offender } => {
                write!(f, "compatibility envelope contains zero ABI {offender}")
            }
            Self::DuplicatesCurrent { duplicate } => {
                write!(
                    f,
                    "compatibility envelope duplicates current ABI {duplicate}"
                )
            }
            Self::BackwardLargerThanCurrent { offender } => {
                write!(
                    f,
                    "backward compatible ABI {offender} is larger than current"
                )
            }
            Self::ForwardSmallerThanCurrent { offender } => {
                write!(
                    f,
                    "forward handshake ABI {offender} is smaller than current"
                )
            }
            Self::DuplicateInBackward { offender } => {
                write!(f, "duplicate backward compatible ABI {offender}")
            }
            Self::DuplicateInForward { offender } => {
                write!(f, "duplicate forward handshake ABI {offender}")
            }
        }
    }
}

#[cfg(all(feature = "host", feature = "std"))]
impl std::error::Error for CompatibilityError {}

/// ROM-resident identity block read by harnesses and emulator adapters.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentityBlock {
    pub magic: [u8; 4],
    pub abi: AbiVersion,
    pub _reserved0: u8,
    pub build_hash: [u8; 32],
    pub artifact_core_hash: [u8; 32],
    pub runtime_nucleus_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub timestamp_unix: u64,
    pub continuation_tail_bytes: u32,
    pub semantic_schema_version: u16,
    pub _reserved1: [u8; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildIdentityArgs {
    pub abi: AbiVersion,
    pub build_hash: [u8; 32],
    pub artifact_core_hash: [u8; 32],
    pub runtime_nucleus_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub timestamp_unix: u64,
    pub continuation_tail_bytes: u32,
    pub semantic_schema_version: u16,
}

impl BuildIdentityBlock {
    pub const MAGIC: [u8; 4] = *b"GBLM";
    pub const SIZE: usize = 152;

    #[must_use]
    pub const fn new(args: BuildIdentityArgs) -> Self {
        Self {
            magic: Self::MAGIC,
            abi: args.abi,
            _reserved0: 0,
            build_hash: args.build_hash,
            artifact_core_hash: args.artifact_core_hash,
            runtime_nucleus_hash: args.runtime_nucleus_hash,
            compile_request_hash: args.compile_request_hash,
            timestamp_unix: args.timestamp_unix,
            continuation_tail_bytes: args.continuation_tail_bytes,
            semantic_schema_version: args.semantic_schema_version,
            _reserved1: [0, 0],
        }
    }

    pub fn validate(&self) -> Result<(), BuildIdentityError> {
        if self.magic != Self::MAGIC {
            return Err(BuildIdentityError::BadMagic {
                observed: self.magic,
                expected: Self::MAGIC,
            });
        }
        if self.abi.is_zero() {
            return Err(BuildIdentityError::BadAbi {
                error: AbiVersionError::Zero,
            });
        }
        let reserved = [
            (offset_of!(BuildIdentityBlock, _reserved0), self._reserved0),
            (
                offset_of!(BuildIdentityBlock, _reserved1),
                self._reserved1[0],
            ),
            (
                offset_of!(BuildIdentityBlock, _reserved1) + 1,
                self._reserved1[1],
            ),
        ];
        for (offset, value) in reserved {
            if value != 0 {
                return Err(BuildIdentityError::NonZeroReserved { offset, value });
            }
        }
        if self.semantic_schema_version == 0 {
            return Err(BuildIdentityError::BadSchemaVersion { observed: 0 });
        }

        Ok(())
    }

    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Result<Self, BuildIdentityError> {
        let block = Self {
            magic: bytes[0..4].try_into().expect("slice length is fixed"),
            abi: AbiVersion {
                major: bytes[4],
                minor: bytes[5],
                patch: bytes[6],
            },
            _reserved0: bytes[7],
            build_hash: bytes[8..40].try_into().expect("slice length is fixed"),
            artifact_core_hash: bytes[40..72].try_into().expect("slice length is fixed"),
            runtime_nucleus_hash: bytes[72..104].try_into().expect("slice length is fixed"),
            compile_request_hash: bytes[104..136].try_into().expect("slice length is fixed"),
            timestamp_unix: u64::from_le_bytes(
                bytes[136..144].try_into().expect("slice length is fixed"),
            ),
            continuation_tail_bytes: u32::from_le_bytes(
                bytes[144..148].try_into().expect("slice length is fixed"),
            ),
            semantic_schema_version: u16::from_le_bytes(
                bytes[148..150].try_into().expect("slice length is fixed"),
            ),
            _reserved1: bytes[150..152].try_into().expect("slice length is fixed"),
        };
        block.validate()?;
        Ok(block)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.magic);
        out[4] = self.abi.major;
        out[5] = self.abi.minor;
        out[6] = self.abi.patch;
        out[7] = self._reserved0;
        out[8..40].copy_from_slice(&self.build_hash);
        out[40..72].copy_from_slice(&self.artifact_core_hash);
        out[72..104].copy_from_slice(&self.runtime_nucleus_hash);
        out[104..136].copy_from_slice(&self.compile_request_hash);
        out[136..144].copy_from_slice(&self.timestamp_unix.to_le_bytes());
        out[144..148].copy_from_slice(&self.continuation_tail_bytes.to_le_bytes());
        out[148..150].copy_from_slice(&self.semantic_schema_version.to_le_bytes());
        out[150..152].copy_from_slice(&self._reserved1);
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildIdentityError {
    BadMagic {
        observed: [u8; 4],
        expected: [u8; 4],
    },
    BadAbi {
        error: AbiVersionError,
    },
    Truncated {
        expected: usize,
        observed: usize,
    },
    NonZeroReserved {
        offset: usize,
        value: u8,
    },
    BadSchemaVersion {
        observed: u16,
    },
}

impl fmt::Display for BuildIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic { observed, expected } => write!(
                f,
                "bad build identity magic {:?}; expected {:?}",
                observed, expected
            ),
            Self::BadAbi { error } => write!(f, "bad build identity ABI: {error}"),
            Self::Truncated { expected, observed } => write!(
                f,
                "build identity block truncated: expected {expected} bytes, observed {observed}"
            ),
            Self::NonZeroReserved { offset, value } => write!(
                f,
                "build identity reserved byte at offset {offset} is non-zero: {value}"
            ),
            Self::BadSchemaVersion { observed } => {
                write!(
                    f,
                    "semantic schema version must be >= 1, observed {observed}"
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BuildIdentityError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> BuildIdentityArgs {
        BuildIdentityArgs {
            abi: CURRENT_ABI,
            build_hash: [1; 32],
            artifact_core_hash: [2; 32],
            runtime_nucleus_hash: [3; 32],
            compile_request_hash: [4; 32],
            timestamp_unix: 1_700_000_000,
            continuation_tail_bytes: 64,
            semantic_schema_version: 1,
        }
    }

    #[test]
    fn current_constant_set() {
        assert!(!CURRENT_ABI.is_zero());
        assert_eq!(CURRENT_ABI, AbiVersion::new(0, 1, 0));
    }

    #[test]
    fn ord_total() {
        assert!(AbiVersion::new(0, 1, 0) < AbiVersion::new(0, 2, 0));
        assert!(AbiVersion::new(0, 2, 0) < AbiVersion::new(1, 0, 0));
    }

    #[test]
    #[cfg(feature = "host")]
    fn semver_round_trip() {
        let semver = gbf_foundation::SemVer::new(255, 1, 2);
        let abi = AbiVersion::from_semver(semver).expect("in range");

        assert_eq!(abi, AbiVersion::new(255, 1, 2));
        assert_eq!(abi.to_semver(), semver);
        assert!(matches!(
            AbiVersion::from_semver(gbf_foundation::SemVer::new(256, 0, 0)),
            Err(AbiVersionError::SemVerOutOfRange { .. })
        ));
    }

    #[test]
    fn abi_version_serde_round_trip() {
        let abi = AbiVersion::new(1, 2, 3);
        let encoded = serde_json::to_string(&abi).expect("ABI version serializes");
        let decoded: AbiVersion = serde_json::from_str(&encoded).expect("ABI version deserializes");

        assert_eq!(decoded, abi);
    }

    #[test]
    #[cfg(feature = "host")]
    fn compatibility_envelope_validate() {
        let envelope = CompatibilityEnvelope {
            current: AbiVersion::new(0, 2, 0),
            backward_compatible_with: alloc::vec![AbiVersion::new(0, 1, 0)],
            forward_handshake: alloc::vec![AbiVersion::new(0, 3, 0)],
        };

        envelope.validate().expect("valid envelope");
        assert!(envelope.accepts(AbiVersion::new(0, 1, 0)));
        assert!(envelope.accepts(AbiVersion::new(0, 2, 0)));
        assert!(envelope.accepts(AbiVersion::new(0, 3, 0)));
        assert!(!envelope.accepts(AbiVersion::new(1, 0, 0)));
    }

    #[test]
    #[cfg(feature = "host")]
    fn compatibility_envelope_no_self() {
        let envelope = CompatibilityEnvelope {
            current: CURRENT_ABI,
            backward_compatible_with: alloc::vec![CURRENT_ABI],
            forward_handshake: alloc::vec![],
        };

        assert_eq!(
            envelope.validate(),
            Err(CompatibilityError::DuplicatesCurrent {
                duplicate: CURRENT_ABI
            })
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn compatibility_envelope_rejects_zero_version() {
        let zero_current = CompatibilityEnvelope::current_only(AbiVersion::new(0, 0, 0));
        assert_eq!(
            zero_current.validate(),
            Err(CompatibilityError::ZeroVersion {
                offender: AbiVersion::new(0, 0, 0)
            })
        );

        let zero_backward = CompatibilityEnvelope {
            current: CURRENT_ABI,
            backward_compatible_with: alloc::vec![AbiVersion::new(0, 0, 0)],
            forward_handshake: alloc::vec![],
        };
        assert_eq!(
            zero_backward.validate(),
            Err(CompatibilityError::ZeroVersion {
                offender: AbiVersion::new(0, 0, 0)
            })
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn compatibility_envelope_serde_round_trip() {
        let envelope = CompatibilityEnvelope {
            current: AbiVersion::new(0, 2, 0),
            backward_compatible_with: alloc::vec![AbiVersion::new(0, 1, 0)],
            forward_handshake: alloc::vec![AbiVersion::new(0, 3, 0)],
        };
        let encoded = serde_json::to_string(&envelope).expect("envelope serializes");
        let decoded: CompatibilityEnvelope =
            serde_json::from_str(&encoded).expect("envelope deserializes");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn build_identity_layout() {
        assert_eq!(size_of::<BuildIdentityBlock>(), BuildIdentityBlock::SIZE);
        assert_eq!(align_of::<BuildIdentityBlock>(), 8);
    }

    #[test]
    fn build_identity_offsets() {
        assert_eq!(offset_of!(BuildIdentityBlock, magic), 0);
        assert_eq!(offset_of!(BuildIdentityBlock, abi), 4);
        assert_eq!(offset_of!(BuildIdentityBlock, build_hash), 8);
        assert_eq!(offset_of!(BuildIdentityBlock, artifact_core_hash), 40);
        assert_eq!(offset_of!(BuildIdentityBlock, runtime_nucleus_hash), 72);
        assert_eq!(offset_of!(BuildIdentityBlock, compile_request_hash), 104);
        assert_eq!(offset_of!(BuildIdentityBlock, timestamp_unix), 136);
        assert_eq!(offset_of!(BuildIdentityBlock, continuation_tail_bytes), 144);
        assert_eq!(offset_of!(BuildIdentityBlock, semantic_schema_version), 148);
        assert_eq!(offset_of!(BuildIdentityBlock, _reserved1), 150);
    }

    #[test]
    fn build_identity_constructor_sets_magic() {
        assert_eq!(BuildIdentityBlock::new(args()).magic, *b"GBLM");
    }

    #[test]
    fn build_identity_constructor_zeroes_reserved() {
        let block = BuildIdentityBlock::new(args());

        assert_eq!(block._reserved0, 0);
        assert_eq!(block._reserved1, [0, 0]);
    }

    #[test]
    fn build_identity_validate_rejects_bad_magic() {
        let mut block = BuildIdentityBlock::new(args());
        block.magic = *b"NOPE";

        assert!(matches!(
            block.validate(),
            Err(BuildIdentityError::BadMagic { .. })
        ));
    }

    #[test]
    fn build_identity_validate_rejects_nonzero_reserved() {
        let mut block = BuildIdentityBlock::new(args());
        block._reserved1[1] = 7;

        assert!(matches!(
            block.validate(),
            Err(BuildIdentityError::NonZeroReserved {
                offset: 151,
                value: 7
            })
        ));
    }

    #[test]
    fn build_identity_validate_rejects_reserved0() {
        let mut block = BuildIdentityBlock::new(args());
        block._reserved0 = 7;

        assert!(matches!(
            block.validate(),
            Err(BuildIdentityError::NonZeroReserved {
                offset: 7,
                value: 7
            })
        ));
    }

    #[test]
    fn build_identity_validate_rejects_reserved1_first_byte() {
        let mut block = BuildIdentityBlock::new(args());
        block._reserved1[0] = 7;

        assert!(matches!(
            block.validate(),
            Err(BuildIdentityError::NonZeroReserved {
                offset: 150,
                value: 7
            })
        ));
    }

    #[test]
    fn build_identity_validate_rejects_zero_abi() {
        let mut block = BuildIdentityBlock::new(args());
        block.abi = AbiVersion::new(0, 0, 0);

        assert!(matches!(
            block.validate(),
            Err(BuildIdentityError::BadAbi {
                error: AbiVersionError::Zero
            })
        ));
    }

    #[test]
    fn build_identity_validate_rejects_zero_schema_version() {
        let mut block = BuildIdentityBlock::new(args());
        block.semantic_schema_version = 0;

        assert_eq!(
            block.validate(),
            Err(BuildIdentityError::BadSchemaVersion { observed: 0 })
        );
    }

    #[test]
    fn build_identity_from_bytes_round_trip() {
        let block = BuildIdentityBlock::new(args());
        let bytes = block.to_bytes();
        let decoded = BuildIdentityBlock::from_bytes(&bytes).expect("bytes validate");

        assert_eq!(decoded, block);
        assert_eq!(decoded.to_bytes(), bytes);
    }

    #[test]
    fn build_identity_serde_round_trip() {
        let block = BuildIdentityBlock::new(args());
        let encoded = serde_json::to_string(&block).expect("identity serializes");
        let decoded: BuildIdentityBlock =
            serde_json::from_str(&encoded).expect("identity deserializes");

        assert_eq!(decoded, block);
    }

    #[test]
    fn build_identity_has_no_drop() {
        assert!(!core::mem::needs_drop::<AbiVersion>());
        assert!(!core::mem::needs_drop::<BuildIdentityBlock>());
    }
}
