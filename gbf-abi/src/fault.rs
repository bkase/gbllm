//! Fault taxonomy, fault snapshots, and host recovery policy types.

use core::fmt;
#[cfg(test)]
use core::mem::{align_of, size_of};
use core::ops::RangeInclusive;

#[cfg(feature = "host")]
use alloc::collections::BTreeMap;
use memoffset::offset_of;
use serde::{Deserialize, Serialize};

use crate::checkpoint::CompactCheckpointId;
use crate::liveness::LivenessCounters;

/// Pinned fault code discriminants grouped by domain ranges.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FaultCode {
    None = 0x0000,
    AbiVersionMismatch = 0x0001,
    BuildIdentityMismatch = 0x0002,
    PersistChecksum = 0x0010,
    PersistTornWrite = 0x0011,
    PersistSchemaUnknown = 0x0012,
    BankShadowDivergence = 0x0020,
    UnauthorizedMbcWrite = 0x0021,
    LeaseUnbalanced = 0x0022,
    YieldTooLong = 0x0030,
    InterruptLatencyExceeded = 0x0031,
    LivenessTimeout = 0x0032,
    RepeatedCheckpointNoProgress = 0x0033,
    UiCommitOutsideLegalMode = 0x0040,
    UnknownChecksumKind = 0x0050,
    UnknownPersistKind = 0x0051,
    HarnessProtocolError = 0x0060,
    TraceBudgetExceeded = 0x0070,
    CalibrationDrift = 0x0080,
    InternalAssertion = 0xFF00,
}

impl FaultCode {
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::AbiVersionMismatch,
        Self::BuildIdentityMismatch,
        Self::PersistChecksum,
        Self::PersistTornWrite,
        Self::PersistSchemaUnknown,
        Self::BankShadowDivergence,
        Self::UnauthorizedMbcWrite,
        Self::LeaseUnbalanced,
        Self::YieldTooLong,
        Self::InterruptLatencyExceeded,
        Self::LivenessTimeout,
        Self::RepeatedCheckpointNoProgress,
        Self::UiCommitOutsideLegalMode,
        Self::UnknownChecksumKind,
        Self::UnknownPersistKind,
        Self::HarnessProtocolError,
        Self::TraceBudgetExceeded,
        Self::CalibrationDrift,
        Self::InternalAssertion,
    ];

    #[must_use]
    pub const fn from_u16(raw: u16) -> Option<Self> {
        match raw {
            0x0000 => Some(Self::None),
            0x0001 => Some(Self::AbiVersionMismatch),
            0x0002 => Some(Self::BuildIdentityMismatch),
            0x0010 => Some(Self::PersistChecksum),
            0x0011 => Some(Self::PersistTornWrite),
            0x0012 => Some(Self::PersistSchemaUnknown),
            0x0020 => Some(Self::BankShadowDivergence),
            0x0021 => Some(Self::UnauthorizedMbcWrite),
            0x0022 => Some(Self::LeaseUnbalanced),
            0x0030 => Some(Self::YieldTooLong),
            0x0031 => Some(Self::InterruptLatencyExceeded),
            0x0032 => Some(Self::LivenessTimeout),
            0x0033 => Some(Self::RepeatedCheckpointNoProgress),
            0x0040 => Some(Self::UiCommitOutsideLegalMode),
            0x0050 => Some(Self::UnknownChecksumKind),
            0x0051 => Some(Self::UnknownPersistKind),
            0x0060 => Some(Self::HarnessProtocolError),
            0x0070 => Some(Self::TraceBudgetExceeded),
            0x0080 => Some(Self::CalibrationDrift),
            0xFF00 => Some(Self::InternalAssertion),
            _ => None,
        }
    }
}

/// Pinned fault domains. Values are written into `FaultSnapshot::domain`.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FaultDomain {
    None = 0x00,
    Boot = 0x01,
    Persistence = 0x02,
    Banking = 0x03,
    Scheduling = 0x04,
    Liveness = 0x05,
    Ui = 0x06,
    Schema = 0x07,
    Harness = 0x08,
    Trace = 0x09,
    Calibration = 0x0A,
    Internal = 0xFF,
}

impl FaultDomain {
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::Boot,
        Self::Persistence,
        Self::Banking,
        Self::Scheduling,
        Self::Liveness,
        Self::Ui,
        Self::Schema,
        Self::Harness,
        Self::Trace,
        Self::Calibration,
        Self::Internal,
    ];

    #[must_use]
    pub const fn from_u16(raw: u16) -> Option<Self> {
        match raw {
            0x00 => Some(Self::None),
            0x01 => Some(Self::Boot),
            0x02 => Some(Self::Persistence),
            0x03 => Some(Self::Banking),
            0x04 => Some(Self::Scheduling),
            0x05 => Some(Self::Liveness),
            0x06 => Some(Self::Ui),
            0x07 => Some(Self::Schema),
            0x08 => Some(Self::Harness),
            0x09 => Some(Self::Trace),
            0x0A => Some(Self::Calibration),
            0xFF => Some(Self::Internal),
            _ => None,
        }
    }

    #[must_use]
    pub const fn range(self) -> RangeInclusive<u16> {
        match self {
            Self::None => 0x0000..=0x0000,
            Self::Boot => 0x0001..=0x000F,
            Self::Persistence => 0x0010..=0x001F,
            Self::Banking => 0x0020..=0x002F,
            Self::Scheduling => 0x0030..=0x0031,
            Self::Liveness => 0x0032..=0x003F,
            Self::Ui => 0x0040..=0x004F,
            Self::Schema => 0x0050..=0x005F,
            Self::Harness => 0x0060..=0x006F,
            Self::Trace => 0x0070..=0x007F,
            Self::Calibration => 0x0080..=0x008F,
            Self::Internal => 0xFF00..=0xFFFF,
        }
    }
}

#[must_use]
pub const fn classify_fault(code: FaultCode) -> FaultDomain {
    match code {
        FaultCode::None => FaultDomain::None,
        FaultCode::AbiVersionMismatch | FaultCode::BuildIdentityMismatch => FaultDomain::Boot,
        FaultCode::PersistChecksum
        | FaultCode::PersistTornWrite
        | FaultCode::PersistSchemaUnknown => FaultDomain::Persistence,
        FaultCode::BankShadowDivergence
        | FaultCode::UnauthorizedMbcWrite
        | FaultCode::LeaseUnbalanced => FaultDomain::Banking,
        FaultCode::YieldTooLong | FaultCode::InterruptLatencyExceeded => FaultDomain::Scheduling,
        FaultCode::LivenessTimeout | FaultCode::RepeatedCheckpointNoProgress => {
            FaultDomain::Liveness
        }
        FaultCode::UiCommitOutsideLegalMode => FaultDomain::Ui,
        FaultCode::UnknownChecksumKind | FaultCode::UnknownPersistKind => FaultDomain::Schema,
        FaultCode::HarnessProtocolError => FaultDomain::Harness,
        FaultCode::TraceBudgetExceeded => FaultDomain::Trace,
        FaultCode::CalibrationDrift => FaultDomain::Calibration,
        FaultCode::InternalAssertion => FaultDomain::Internal,
    }
}

/// LR35902 register subset captured in fault snapshots.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSnapshot {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
}

/// SRAM-resident record written on fault.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultSnapshot {
    pub code: u16,
    pub domain: u16,
    pub at_pc: u16,
    pub at_bank: u16,
    pub at_checkpoint: CompactCheckpointId,
    pub _resv: u16,
    pub regs: RegisterSnapshot,
    pub _resv1: [u8; 2],
    pub liveness: LivenessCounters,
}

impl FaultSnapshot {
    pub const SIZE: usize = 36;

    #[must_use]
    pub const fn new(
        code: FaultCode,
        at_pc: u16,
        at_bank: u16,
        at_checkpoint: CompactCheckpointId,
        regs: RegisterSnapshot,
        liveness: LivenessCounters,
    ) -> Self {
        Self {
            code: code as u16,
            domain: classify_fault(code) as u16,
            at_pc,
            at_bank,
            at_checkpoint,
            _resv: 0,
            regs,
            _resv1: [0, 0],
            liveness,
        }
    }

    pub fn code_decoded(&self) -> Result<FaultCode, SnapshotDecodeError> {
        FaultCode::from_u16(self.code)
            .ok_or(SnapshotDecodeError::UnknownFaultCode { raw: self.code })
    }

    pub fn domain_decoded(&self) -> Result<FaultDomain, SnapshotDecodeError> {
        FaultDomain::from_u16(self.domain)
            .ok_or(SnapshotDecodeError::UnknownFaultDomain { raw: self.domain })
    }

    pub fn validate(&self) -> Result<(), SnapshotDecodeError> {
        let resv = self._resv.to_le_bytes();
        let liveness_reserved_offset =
            offset_of!(FaultSnapshot, liveness) + offset_of!(LivenessCounters, _reserved);
        let reserved = [
            (offset_of!(FaultSnapshot, _resv), resv[0]),
            (offset_of!(FaultSnapshot, _resv) + 1, resv[1]),
            (offset_of!(FaultSnapshot, _resv1), self._resv1[0]),
            (offset_of!(FaultSnapshot, _resv1) + 1, self._resv1[1]),
            (liveness_reserved_offset, self.liveness._reserved[0]),
            (liveness_reserved_offset + 1, self.liveness._reserved[1]),
        ];
        for (offset, value) in reserved {
            if value != 0 {
                return Err(SnapshotDecodeError::NonZeroReserved { offset, value });
            }
        }

        let code = self.code_decoded()?;
        let domain = self.domain_decoded()?;
        let expected = classify_fault(code);
        if domain != expected {
            return Err(SnapshotDecodeError::DomainMismatch {
                code,
                observed: domain,
                expected,
            });
        }

        Ok(())
    }

    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Result<Self, SnapshotDecodeError> {
        let snapshot = Self {
            code: u16::from_le_bytes(bytes[0..2].try_into().expect("slice length is fixed")),
            domain: u16::from_le_bytes(bytes[2..4].try_into().expect("slice length is fixed")),
            at_pc: u16::from_le_bytes(bytes[4..6].try_into().expect("slice length is fixed")),
            at_bank: u16::from_le_bytes(bytes[6..8].try_into().expect("slice length is fixed")),
            at_checkpoint: CompactCheckpointId(u16::from_le_bytes(
                bytes[8..10].try_into().expect("slice length is fixed"),
            )),
            _resv: u16::from_le_bytes(bytes[10..12].try_into().expect("slice length is fixed")),
            regs: RegisterSnapshot {
                a: bytes[12],
                f: bytes[13],
                b: bytes[14],
                c: bytes[15],
                d: bytes[16],
                e: bytes[17],
                h: bytes[18],
                l: bytes[19],
                sp: u16::from_le_bytes(bytes[20..22].try_into().expect("slice length is fixed")),
            },
            _resv1: bytes[22..24].try_into().expect("slice length is fixed"),
            liveness: LivenessCounters {
                progress_epoch: u32::from_le_bytes(
                    bytes[24..28].try_into().expect("slice length is fixed"),
                ),
                last_checkpoint: CompactCheckpointId(u16::from_le_bytes(
                    bytes[28..30].try_into().expect("slice length is fixed"),
                )),
                no_progress_frames: u16::from_le_bytes(
                    bytes[30..32].try_into().expect("slice length is fixed"),
                ),
                livelock_threshold_frames: u16::from_le_bytes(
                    bytes[32..34].try_into().expect("slice length is fixed"),
                ),
                _reserved: bytes[34..36].try_into().expect("slice length is fixed"),
            },
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..2].copy_from_slice(&self.code.to_le_bytes());
        out[2..4].copy_from_slice(&self.domain.to_le_bytes());
        out[4..6].copy_from_slice(&self.at_pc.to_le_bytes());
        out[6..8].copy_from_slice(&self.at_bank.to_le_bytes());
        out[8..10].copy_from_slice(&self.at_checkpoint.0.to_le_bytes());
        out[10..12].copy_from_slice(&self._resv.to_le_bytes());
        out[12] = self.regs.a;
        out[13] = self.regs.f;
        out[14] = self.regs.b;
        out[15] = self.regs.c;
        out[16] = self.regs.d;
        out[17] = self.regs.e;
        out[18] = self.regs.h;
        out[19] = self.regs.l;
        out[20..22].copy_from_slice(&self.regs.sp.to_le_bytes());
        out[22..24].copy_from_slice(&self._resv1);
        out[24..28].copy_from_slice(&self.liveness.progress_epoch.to_le_bytes());
        out[28..30].copy_from_slice(&self.liveness.last_checkpoint.0.to_le_bytes());
        out[30..32].copy_from_slice(&self.liveness.no_progress_frames.to_le_bytes());
        out[32..34].copy_from_slice(&self.liveness.livelock_threshold_frames.to_le_bytes());
        out[34..36].copy_from_slice(&self.liveness._reserved);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotDecodeError {
    UnknownFaultCode {
        raw: u16,
    },
    UnknownFaultDomain {
        raw: u16,
    },
    DomainMismatch {
        code: FaultCode,
        observed: FaultDomain,
        expected: FaultDomain,
    },
    NonZeroReserved {
        offset: usize,
        value: u8,
    },
}

impl fmt::Display for SnapshotDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFaultCode { raw } => write!(f, "unknown fault code 0x{raw:04x}"),
            Self::UnknownFaultDomain { raw } => write!(f, "unknown fault domain 0x{raw:04x}"),
            Self::DomainMismatch {
                code,
                observed,
                expected,
            } => write!(
                f,
                "fault snapshot domain mismatch for {code:?}: observed {observed:?}, expected {expected:?}"
            ),
            Self::NonZeroReserved { offset, value } => {
                write!(
                    f,
                    "fault snapshot reserved byte {offset} is non-zero: {value}"
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SnapshotDecodeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    ColdStart,
    DemoteToSafeMode,
    AbortAndPanic,
    BootValidationOnly,
    RetrySlice,
    DropTrace,
    HardReset,
}

impl RecoveryAction {
    pub const ALL: &'static [Self] = &[
        Self::ColdStart,
        Self::DemoteToSafeMode,
        Self::AbortAndPanic,
        Self::BootValidationOnly,
        Self::RetrySlice,
        Self::DropTrace,
        Self::HardReset,
    ];
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultPolicy {
    pub by_domain: BTreeMap<FaultDomain, RecoveryAction>,
    pub default_action: RecoveryAction,
}

#[cfg(feature = "host")]
impl FaultPolicy {
    #[must_use]
    pub fn action_for(&self, code: FaultCode) -> RecoveryAction {
        self.by_domain
            .get(&classify_fault(code))
            .copied()
            .unwrap_or(self.default_action)
    }

    pub fn validate(&self) -> Result<(), FaultPolicyError> {
        if self.default_action == RecoveryAction::BootValidationOnly {
            return Err(FaultPolicyError::DefaultActionIsBootValidationOnly);
        }

        Ok(())
    }
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultPolicyError {
    DefaultActionIsBootValidationOnly,
}

#[cfg(feature = "host")]
impl fmt::Display for FaultPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultActionIsBootValidationOnly => {
                f.write_str("FaultPolicy.default_action must not be BootValidationOnly")
            }
        }
    }
}

#[cfg(all(feature = "host", feature = "std"))]
impl std::error::Error for FaultPolicyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootValidationPlan {
    pub validate_persistence: bool,
    pub validate_runtime_nucleus_hash: bool,
    pub validate_artifact_core_hash: bool,
    pub validate_compile_request_hash: bool,
    pub persist_scan_policy: PersistScanPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistScanPolicy {
    StrictCriticalOnly,
    ScanAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs() -> RegisterSnapshot {
        RegisterSnapshot {
            a: 1,
            f: 2,
            b: 3,
            c: 4,
            d: 5,
            e: 6,
            h: 7,
            l: 8,
            sp: 0xFFFE,
        }
    }

    #[test]
    fn all_unique_discriminants() {
        for (index, code) in FaultCode::ALL.iter().enumerate() {
            for other in &FaultCode::ALL[index + 1..] {
                assert_ne!(*code as u16, *other as u16);
            }
        }
    }

    #[test]
    fn code_from_u16_round_trip() {
        for code in FaultCode::ALL {
            assert_eq!(FaultCode::from_u16(*code as u16), Some(*code));
        }
        assert_eq!(FaultCode::from_u16(0x00FE), None);
    }

    #[test]
    fn code_to_domain_total() {
        for code in FaultCode::ALL {
            assert!(FaultDomain::ALL.contains(&classify_fault(*code)));
        }
    }

    #[test]
    fn range_partition() {
        for code in FaultCode::ALL {
            let raw = *code as u16;
            assert!(classify_fault(*code).range().contains(&raw));
        }
    }

    #[test]
    fn register_snapshot_layout() {
        assert_eq!(size_of::<RegisterSnapshot>(), 10);
        assert_eq!(align_of::<RegisterSnapshot>(), 2);
        assert_eq!(offset_of!(RegisterSnapshot, a), 0);
        assert_eq!(offset_of!(RegisterSnapshot, f), 1);
        assert_eq!(offset_of!(RegisterSnapshot, b), 2);
        assert_eq!(offset_of!(RegisterSnapshot, c), 3);
        assert_eq!(offset_of!(RegisterSnapshot, d), 4);
        assert_eq!(offset_of!(RegisterSnapshot, e), 5);
        assert_eq!(offset_of!(RegisterSnapshot, h), 6);
        assert_eq!(offset_of!(RegisterSnapshot, l), 7);
        assert_eq!(offset_of!(RegisterSnapshot, sp), 8);
    }

    #[test]
    fn snapshot_layout() {
        assert_eq!(size_of::<FaultSnapshot>(), 36);
        assert_eq!(align_of::<FaultSnapshot>(), 4);
        assert_eq!(offset_of!(FaultSnapshot, liveness), 24);
    }

    #[test]
    fn snapshot_constructor_zeroes_reserved() {
        let snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );

        assert_eq!(snapshot._resv, 0);
        assert_eq!(snapshot._resv1, [0, 0]);
    }

    #[test]
    fn snapshot_domain_matches_code() {
        let snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );

        snapshot.validate().expect("snapshot validates");
        assert_eq!(snapshot.domain_decoded(), Ok(FaultDomain::Liveness));
    }

    #[test]
    fn snapshot_validate_rejects_unknown_code() {
        let mut snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        snapshot.code = 0xDEAD;

        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::UnknownFaultCode { raw: 0xDEAD })
        );
    }

    #[test]
    fn snapshot_validate_rejects_unknown_domain() {
        let mut snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        snapshot.domain = 0x00FE;

        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::UnknownFaultDomain { raw: 0x00FE })
        );
    }

    #[test]
    fn snapshot_validate_rejects_domain_mismatch() {
        let mut snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        snapshot.domain = FaultDomain::Harness as u16;

        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::DomainMismatch {
                code: FaultCode::LivenessTimeout,
                observed: FaultDomain::Harness,
                expected: FaultDomain::Liveness
            })
        );
    }

    #[test]
    fn snapshot_validate_rejects_reserved_bytes() {
        let mut snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        snapshot._resv = 0x0100;
        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::NonZeroReserved {
                offset: 11,
                value: 1
            })
        );

        snapshot._resv = 0;
        snapshot._resv1[1] = 2;
        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::NonZeroReserved {
                offset: 23,
                value: 2
            })
        );

        snapshot._resv1[1] = 0;
        snapshot.liveness._reserved[0] = 3;
        assert_eq!(
            snapshot.validate(),
            Err(SnapshotDecodeError::NonZeroReserved {
                offset: 34,
                value: 3
            })
        );
    }

    #[test]
    fn snapshot_from_bytes_rejects_nested_liveness_reserved_bytes() {
        let snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        let mut bytes = snapshot.to_bytes();
        bytes[35] = 4;

        assert_eq!(
            FaultSnapshot::from_bytes(&bytes),
            Err(SnapshotDecodeError::NonZeroReserved {
                offset: 35,
                value: 4
            })
        );
    }

    #[test]
    fn snapshot_from_bytes_round_trip() {
        let mut liveness = LivenessCounters::new(60);
        liveness.record_progress(CompactCheckpointId(3));
        let snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            liveness,
        );
        let bytes = snapshot.to_bytes();
        let decoded = FaultSnapshot::from_bytes(&bytes).expect("bytes validate");

        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.to_bytes(), bytes);
    }

    #[test]
    fn snapshot_serde_round_trip() {
        let snapshot = FaultSnapshot::new(
            FaultCode::LivenessTimeout,
            0x1234,
            3,
            CompactCheckpointId(9),
            regs(),
            LivenessCounters::new(60),
        );
        let encoded = serde_json::to_string(&snapshot).expect("snapshot serializes");
        let decoded: FaultSnapshot = serde_json::from_str(&encoded).expect("snapshot deserializes");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn snapshot_has_no_drop() {
        assert!(!core::mem::needs_drop::<RegisterSnapshot>());
        assert!(!core::mem::needs_drop::<FaultSnapshot>());
    }

    #[test]
    fn recovery_action_exhaustive() {
        assert_eq!(RecoveryAction::ALL.len(), 7);
        assert!(RecoveryAction::ALL.contains(&RecoveryAction::ColdStart));
        assert!(RecoveryAction::ALL.contains(&RecoveryAction::HardReset));
    }

    #[test]
    #[cfg(feature = "host")]
    fn policy_default_action_validation() {
        let policy = FaultPolicy {
            by_domain: BTreeMap::new(),
            default_action: RecoveryAction::BootValidationOnly,
        };

        assert_eq!(
            policy.validate(),
            Err(FaultPolicyError::DefaultActionIsBootValidationOnly)
        );
    }

    #[test]
    #[cfg(feature = "host")]
    fn policy_action_for_falls_back_to_default() {
        let mut by_domain = BTreeMap::new();
        by_domain.insert(FaultDomain::Trace, RecoveryAction::DropTrace);
        let policy = FaultPolicy {
            by_domain,
            default_action: RecoveryAction::AbortAndPanic,
        };

        assert_eq!(
            policy.action_for(FaultCode::TraceBudgetExceeded),
            RecoveryAction::DropTrace
        );
        assert_eq!(
            policy.action_for(FaultCode::LivenessTimeout),
            RecoveryAction::AbortAndPanic
        );
    }
}
