//! Fixed continuation header plus helpers for opaque per-build tails.

use core::fmt;
#[cfg(test)]
use core::mem::align_of;
use core::mem::size_of;

use memoffset::offset_of;
use serde::{Deserialize, Serialize};

use crate::fault::FaultCode;
use crate::interrupt::SliceId;
use crate::liveness::LivenessCounters;
use crate::version::{AbiVersion, CURRENT_ABI};

/// Sentinel-coded optional fault for stable `repr(C)` storage.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultCodeOptional(pub u16);

impl FaultCodeOptional {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn from_option(code: Option<FaultCode>) -> Self {
        match code {
            Some(code) => Self(code as u16),
            None => Self::NONE,
        }
    }

    pub fn decode(self) -> Result<Option<FaultCode>, UnknownFaultCode> {
        if self.0 == 0 {
            return Ok(None);
        }

        FaultCode::from_u16(self.0)
            .map(Some)
            .ok_or(UnknownFaultCode { raw: self.0 })
    }

    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownFaultCode {
    pub raw: u16,
}

impl fmt::Display for UnknownFaultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown fault code 0x{:04x}", self.raw)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for UnknownFaultCode {}

/// Stable prefix of an inference continuation; the per-build tail is opaque.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceStateHeader {
    pub abi: AbiVersion,
    pub _reserved0: u8,
    pub schema: u16,
    pub last_fault: FaultCodeOptional,
    pub session_id: u32,
    pub token_count: u32,
    pub slice_id: SliceId,
    pub liveness: LivenessCounters,
}

impl InferenceStateHeader {
    #[must_use]
    pub const fn new(schema: u16, session_id: u32, livelock_threshold_frames: u16) -> Self {
        Self {
            abi: CURRENT_ABI,
            _reserved0: 0,
            schema,
            last_fault: FaultCodeOptional::NONE,
            session_id,
            token_count: 0,
            slice_id: SliceId(0),
            liveness: LivenessCounters::new(livelock_threshold_frames),
        }
    }

    pub fn validate(&self) -> Result<(), ContinuationError> {
        if self.abi != CURRENT_ABI {
            return Err(ContinuationError::BadAbi {
                observed: self.abi,
                expected: CURRENT_ABI,
            });
        }
        if self._reserved0 != 0 {
            return Err(ContinuationError::NonZeroReserved {
                offset: offset_of!(InferenceStateHeader, _reserved0),
                value: self._reserved0,
            });
        }
        if self.schema == 0 {
            return Err(ContinuationError::BadSchemaVersion {
                observed: self.schema,
                expected: 1,
            });
        }
        if let Err(error) = self.last_fault.decode() {
            return Err(ContinuationError::UnknownFaultCodeInLastFault { raw: error.raw });
        }
        for (index, value) in self.liveness._reserved.iter().copied().enumerate() {
            if value != 0 {
                return Err(ContinuationError::NonZeroReserved {
                    offset: offset_of!(InferenceStateHeader, liveness)
                        + offset_of!(LivenessCounters, _reserved)
                        + index,
                    value,
                });
            }
        }

        Ok(())
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0] = self.abi.major;
        out[1] = self.abi.minor;
        out[2] = self.abi.patch;
        out[3] = self._reserved0;
        out[4..6].copy_from_slice(&self.schema.to_le_bytes());
        out[6..8].copy_from_slice(&self.last_fault.raw().to_le_bytes());
        out[8..12].copy_from_slice(&self.session_id.to_le_bytes());
        out[12..16].copy_from_slice(&self.token_count.to_le_bytes());
        out[16..20].copy_from_slice(&self.slice_id.0.to_le_bytes());
        out[20..32].copy_from_slice(&self.liveness.to_bytes());
        out
    }
}

#[must_use]
pub const fn header_size_bytes() -> usize {
    size_of::<InferenceStateHeader>()
}

pub fn total_continuation_bytes(tail: u32) -> Result<usize, ContinuationError> {
    let tail_usize = usize::try_from(tail).map_err(|_| ContinuationError::TailTooLarge { tail })?;
    header_size_bytes()
        .checked_add(tail_usize)
        .ok_or(ContinuationError::TotalSizeOverflow { tail })
}

pub fn decode_header(buf: &[u8]) -> Result<InferenceStateHeader, ContinuationError> {
    if buf.len() < header_size_bytes() {
        return Err(ContinuationError::HeaderTruncated {
            observed: buf.len(),
        });
    }

    let header = InferenceStateHeader {
        abi: AbiVersion {
            major: buf[0],
            minor: buf[1],
            patch: buf[2],
        },
        _reserved0: buf[3],
        schema: u16::from_le_bytes(buf[4..6].try_into().expect("slice length checked")),
        last_fault: FaultCodeOptional(u16::from_le_bytes(
            buf[6..8].try_into().expect("slice length checked"),
        )),
        session_id: u32::from_le_bytes(buf[8..12].try_into().expect("slice length checked")),
        token_count: u32::from_le_bytes(buf[12..16].try_into().expect("slice length checked")),
        slice_id: SliceId(u32::from_le_bytes(
            buf[16..20].try_into().expect("slice length checked"),
        )),
        liveness: LivenessCounters::from_bytes(
            buf[20..32].try_into().expect("slice length checked"),
        ),
    };

    header.validate()?;
    Ok(header)
}

pub fn split_header_tail(
    buf: &[u8],
    expected_tail_bytes: u32,
) -> Result<(InferenceStateHeader, &[u8]), ContinuationError> {
    let header = decode_header(buf)?;
    let total = total_continuation_bytes(expected_tail_bytes)?;
    if buf.len() < total {
        return Err(ContinuationError::TailTruncated {
            expected_tail: expected_tail_bytes,
            observed: buf.len().saturating_sub(header_size_bytes()),
        });
    }
    if buf.len() > total {
        return Err(ContinuationError::TailLengthMismatch {
            expected_tail: expected_tail_bytes,
            observed: buf.len() - header_size_bytes(),
        });
    }

    Ok((header, &buf[header_size_bytes()..total]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationError {
    HeaderTruncated {
        observed: usize,
    },
    TailTruncated {
        expected_tail: u32,
        observed: usize,
    },
    TailLengthMismatch {
        expected_tail: u32,
        observed: usize,
    },
    TailTooLarge {
        tail: u32,
    },
    TotalSizeOverflow {
        tail: u32,
    },
    BadAbi {
        observed: AbiVersion,
        expected: AbiVersion,
    },
    UnknownFaultCodeInLastFault {
        raw: u16,
    },
    BadSchemaVersion {
        observed: u16,
        expected: u16,
    },
    NonZeroReserved {
        offset: usize,
        value: u8,
    },
}

impl fmt::Display for ContinuationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTruncated { observed } => write!(
                f,
                "inference state header truncated: observed {observed} bytes"
            ),
            Self::TailTruncated {
                expected_tail,
                observed,
            } => write!(
                f,
                "inference state tail truncated: expected {expected_tail} bytes, observed {observed}"
            ),
            Self::TailLengthMismatch {
                expected_tail,
                observed,
            } => write!(
                f,
                "inference state tail length mismatch: expected {expected_tail} bytes, observed {observed}"
            ),
            Self::TailTooLarge { tail } => {
                write!(f, "continuation tail {tail} does not fit usize")
            }
            Self::TotalSizeOverflow { tail } => {
                write!(f, "continuation total size overflows for tail {tail}")
            }
            Self::BadAbi { observed, expected } => {
                write!(f, "bad continuation ABI {observed}; expected {expected}")
            }
            Self::UnknownFaultCodeInLastFault { raw } => {
                write!(f, "unknown last_fault code 0x{raw:04x}")
            }
            Self::BadSchemaVersion { observed, expected } => write!(
                f,
                "bad continuation schema version {observed}; expected at least {expected}"
            ),
            Self::NonZeroReserved { offset, value } => write!(
                f,
                "continuation reserved byte at offset {offset} is non-zero: {value}"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ContinuationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::CompactCheckpointId;

    #[test]
    fn header_layout() {
        assert_eq!(header_size_bytes(), 32);
        assert_eq!(align_of::<InferenceStateHeader>(), 4);
        assert_eq!(offset_of!(InferenceStateHeader, abi), 0);
        assert_eq!(offset_of!(InferenceStateHeader, schema), 4);
        assert_eq!(offset_of!(InferenceStateHeader, last_fault), 6);
        assert_eq!(offset_of!(InferenceStateHeader, liveness), 20);
    }

    #[test]
    fn header_serde_round_trip() {
        let mut header = InferenceStateHeader::new(1, 42, 60);
        header.token_count = 7;
        header.slice_id = SliceId(11);
        header.liveness.record_progress(CompactCheckpointId(3));

        let encoded = serde_json::to_string(&header).expect("header serializes");
        let decoded: InferenceStateHeader =
            serde_json::from_str(&encoded).expect("header deserializes");

        assert_eq!(decoded, header);
    }

    #[test]
    fn split_header_tail_validates_size() {
        let header = InferenceStateHeader::new(1, 42, 60);
        let mut bytes = header.to_bytes().to_vec();
        bytes.extend_from_slice(&[1, 2, 3, 4]);

        let (decoded, tail) = split_header_tail(&bytes, 4).expect("tail length matches");
        assert_eq!(decoded, header);
        assert_eq!(tail, &[1, 2, 3, 4]);

        assert_eq!(
            split_header_tail(&bytes, 5),
            Err(ContinuationError::TailTruncated {
                expected_tail: 5,
                observed: 4
            })
        );

        assert_eq!(
            split_header_tail(&bytes, 3),
            Err(ContinuationError::TailLengthMismatch {
                expected_tail: 3,
                observed: 4
            })
        );
    }

    #[test]
    fn split_header_tail_reports_header_truncated_first() {
        assert_eq!(
            split_header_tail(&[0; 4], 4),
            Err(ContinuationError::HeaderTruncated { observed: 4 })
        );
    }

    #[test]
    fn header_constructor_zeroes_reserved() {
        let header = InferenceStateHeader::new(1, 42, 60);

        assert_eq!(header._reserved0, 0);
        assert_eq!(header.liveness._reserved, [0, 0]);
    }

    #[test]
    fn liveness_reserved_reports_second_byte() {
        let mut header = InferenceStateHeader::new(1, 42, 60);
        header.liveness._reserved[1] = 9;
        let bytes = header.to_bytes();

        assert_eq!(
            decode_header(&bytes),
            Err(ContinuationError::NonZeroReserved {
                offset: 31,
                value: 9
            })
        );
    }

    #[test]
    fn header_has_no_drop() {
        assert!(!core::mem::needs_drop::<InferenceStateHeader>());
        assert!(!core::mem::needs_drop::<FaultCodeOptional>());
    }

    #[test]
    fn fault_code_optional_rejects_unknown() {
        assert_eq!(FaultCodeOptional::NONE.decode(), Ok(None));
        assert_eq!(
            FaultCodeOptional::from_option(Some(FaultCode::LivenessTimeout)).decode(),
            Ok(Some(FaultCode::LivenessTimeout))
        );
        assert_eq!(
            FaultCodeOptional(0xDEAD).decode(),
            Err(UnknownFaultCode { raw: 0xDEAD })
        );
    }
}
