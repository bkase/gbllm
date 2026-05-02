//! Harness command/result control-plane blocks.

use core::fmt;
#[cfg(test)]
use core::mem::{align_of, size_of};

#[cfg(test)]
use memoffset::offset_of;
use serde::{Deserialize, Serialize};

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessOp {
    Nop = 0,
    StepSlice = 1,
    RunUntilCheckpoint = 2,
    DumpArena = 3,
    InjectFault = 4,
    PowerCut = 5,
    SetSession = 6,
    GetState = 7,
}

impl HarnessOp {
    pub const ALL: [Self; 8] = [
        Self::Nop,
        Self::StepSlice,
        Self::RunUntilCheckpoint,
        Self::DumpArena,
        Self::InjectFault,
        Self::PowerCut,
        Self::SetSession,
        Self::GetState,
    ];

    #[must_use]
    pub const fn from_u16(raw: u16) -> Option<Self> {
        match raw {
            0 => Some(Self::Nop),
            1 => Some(Self::StepSlice),
            2 => Some(Self::RunUntilCheckpoint),
            3 => Some(Self::DumpArena),
            4 => Some(Self::InjectFault),
            5 => Some(Self::PowerCut),
            6 => Some(Self::SetSession),
            7 => Some(Self::GetState),
            _ => None,
        }
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessResultKind {
    Ok = 0,
    Done = 1,
    Fault = 2,
    NotImplemented = 3,
    InvalidArgs = 4,
}

impl HarnessResultKind {
    pub const ALL: [Self; 5] = [
        Self::Ok,
        Self::Done,
        Self::Fault,
        Self::NotImplemented,
        Self::InvalidArgs,
    ];

    #[must_use]
    pub const fn from_u16(raw: u16) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Done),
            2 => Some(Self::Fault),
            3 => Some(Self::NotImplemented),
            4 => Some(Self::InvalidArgs),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCommandBlock {
    pub magic: [u8; 4],
    pub seq: u32,
    pub op: u16,
    pub doorbell: u8,
    pub _resv: u8,
    pub args: [u8; 32],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessResultBlock {
    pub magic: [u8; 4],
    pub seq: u32,
    pub kind: u16,
    pub ready: u8,
    pub _resv: u8,
    pub data: [u8; 32],
}

impl HarnessCommandBlock {
    pub const MAGIC: [u8; 4] = *b"HCMD";

    #[must_use]
    pub const fn new(seq: u32, op: HarnessOp, args: [u8; 32]) -> Self {
        Self {
            magic: Self::MAGIC,
            seq,
            op: op as u16,
            doorbell: doorbell::DOORBELL_CLEAR,
            _resv: 0,
            args,
        }
    }

    pub fn raise_doorbell(&mut self) {
        self.doorbell = doorbell::DOORBELL_RAISED;
    }

    pub fn clear_doorbell(&mut self) {
        self.doorbell = doorbell::DOORBELL_CLEAR;
    }

    pub fn decode_op(&self) -> Result<HarnessOp, HarnessProtocolError> {
        HarnessOp::from_u16(self.op).ok_or(HarnessProtocolError::UnknownOp { raw: self.op })
    }

    pub fn validate(&self) -> Result<(), HarnessProtocolError> {
        if self.magic != Self::MAGIC {
            return Err(HarnessProtocolError::BadMagic {
                observed: self.magic,
                expected: Self::MAGIC,
            });
        }
        if self._resv != 0 {
            return Err(HarnessProtocolError::NonZeroReserved { value: self._resv });
        }
        if !doorbell::is_valid_signal(self.doorbell) {
            return Err(HarnessProtocolError::NonZeroReserved {
                value: self.doorbell,
            });
        }
        self.decode_op()?;
        Ok(())
    }
}

impl HarnessResultBlock {
    pub const MAGIC: [u8; 4] = *b"HRES";

    #[must_use]
    pub const fn new(seq: u32, kind: HarnessResultKind, data: [u8; 32]) -> Self {
        Self {
            magic: Self::MAGIC,
            seq,
            kind: kind as u16,
            ready: doorbell::DOORBELL_CLEAR,
            _resv: 0,
            data,
        }
    }

    pub fn mark_ready(&mut self) {
        self.ready = doorbell::DOORBELL_RAISED;
    }

    pub fn clear_ready(&mut self) {
        self.ready = doorbell::DOORBELL_CLEAR;
    }

    pub fn decode_kind(&self) -> Result<HarnessResultKind, HarnessProtocolError> {
        HarnessResultKind::from_u16(self.kind)
            .ok_or(HarnessProtocolError::UnknownResultKind { raw: self.kind })
    }

    pub fn validate_for_command(&self, command_seq: u32) -> Result<(), HarnessProtocolError> {
        if self.magic != Self::MAGIC {
            return Err(HarnessProtocolError::BadMagic {
                observed: self.magic,
                expected: Self::MAGIC,
            });
        }
        if self._resv != 0 {
            return Err(HarnessProtocolError::NonZeroReserved { value: self._resv });
        }
        if !doorbell::is_valid_signal(self.ready) {
            return Err(HarnessProtocolError::NonZeroReserved { value: self.ready });
        }
        self.decode_kind()?;
        if self.seq != command_seq {
            return Err(HarnessProtocolError::SeqMismatch {
                command: command_seq,
                result: self.seq,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessProtocolError {
    BadMagic {
        observed: [u8; 4],
        expected: [u8; 4],
    },
    NonZeroReserved {
        value: u8,
    },
    UnknownOp {
        raw: u16,
    },
    UnknownResultKind {
        raw: u16,
    },
    SeqMismatch {
        command: u32,
        result: u32,
    },
}

impl fmt::Display for HarnessProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic { observed, expected } => {
                write!(
                    f,
                    "bad harness magic {:?}; expected {:?}",
                    observed, expected
                )
            }
            Self::NonZeroReserved { value } => {
                write!(f, "harness reserved byte/value is non-zero: {value}")
            }
            Self::UnknownOp { raw } => write!(f, "unknown harness op {raw}"),
            Self::UnknownResultKind { raw } => write!(f, "unknown harness result kind {raw}"),
            Self::SeqMismatch { command, result } => write!(
                f,
                "harness result seq {result} does not match command seq {command}"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for HarnessProtocolError {}

pub mod doorbell {
    pub const COMMAND_DOORBELL_OFFSET: usize = 10;
    pub const RESULT_READY_OFFSET: usize = 10;
    pub const DOORBELL_RAISED: u8 = 1;
    pub const DOORBELL_CLEAR: u8 = 0;

    #[must_use]
    pub const fn is_valid_signal(value: u8) -> bool {
        matches!(value, DOORBELL_CLEAR | DOORBELL_RAISED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(size_of::<HarnessCommandBlock>(), 44);
        assert_eq!(size_of::<HarnessResultBlock>(), 44);
        assert_eq!(align_of::<HarnessCommandBlock>(), 4);
        assert_eq!(offset_of!(HarnessCommandBlock, doorbell), 10);
        assert_eq!(offset_of!(HarnessCommandBlock, args), 12);
        assert_eq!(offset_of!(HarnessResultBlock, ready), 10);
        assert_eq!(offset_of!(HarnessResultBlock, data), 12);
    }

    #[test]
    fn constructor_sets_magic() {
        let command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        let result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);

        assert_eq!(command.magic, *b"HCMD");
        assert_eq!(result.magic, *b"HRES");
    }

    #[test]
    fn constructor_zeroes_reserved() {
        let command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        let result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);

        assert_eq!(command._resv, 0);
        assert_eq!(result._resv, 0);
    }

    #[test]
    fn constructor_stages_signals_clear() {
        let command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        let result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);

        assert_eq!(command.doorbell, doorbell::DOORBELL_CLEAR);
        assert_eq!(result.ready, doorbell::DOORBELL_CLEAR);
    }

    #[test]
    fn signal_helpers_raise_and_clear() {
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command.raise_doorbell();
        assert_eq!(command.doorbell, doorbell::DOORBELL_RAISED);
        command.clear_doorbell();
        assert_eq!(command.doorbell, doorbell::DOORBELL_CLEAR);

        let mut result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);
        result.mark_ready();
        assert_eq!(result.ready, doorbell::DOORBELL_RAISED);
        result.clear_ready();
        assert_eq!(result.ready, doorbell::DOORBELL_CLEAR);
    }

    #[test]
    fn validate_rejects_bad_magic() {
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command.magic = *b"BAD!";

        assert!(matches!(
            command.validate(),
            Err(HarnessProtocolError::BadMagic { .. })
        ));
    }

    #[test]
    fn validate_rejects_nonzero_reserved() {
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command._resv = 9;

        assert_eq!(
            command.validate(),
            Err(HarnessProtocolError::NonZeroReserved { value: 9 })
        );
    }

    #[test]
    fn validate_rejects_invalid_signal_value() {
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command.doorbell = 2;

        assert_eq!(
            command.validate(),
            Err(HarnessProtocolError::NonZeroReserved { value: 2 })
        );

        let mut result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);
        result.ready = 3;

        assert_eq!(
            result.validate_for_command(1),
            Err(HarnessProtocolError::NonZeroReserved { value: 3 })
        );
    }

    #[test]
    fn result_validate_rejects_bad_magic() {
        let mut result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);
        result.magic = *b"BAD!";

        assert!(matches!(
            result.validate_for_command(1),
            Err(HarnessProtocolError::BadMagic { .. })
        ));
    }

    #[test]
    fn result_validate_rejects_nonzero_reserved() {
        let mut result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);
        result._resv = 9;

        assert_eq!(
            result.validate_for_command(1),
            Err(HarnessProtocolError::NonZeroReserved { value: 9 })
        );
    }

    #[test]
    fn op_kind_complete() {
        for (raw, op) in HarnessOp::ALL.iter().copied().enumerate() {
            assert_eq!(HarnessOp::from_u16(raw as u16), Some(op));
        }
        assert_eq!(HarnessOp::ALL.len(), 8);
    }

    #[test]
    fn result_kind_complete() {
        for (raw, kind) in HarnessResultKind::ALL.iter().copied().enumerate() {
            assert_eq!(HarnessResultKind::from_u16(raw as u16), Some(kind));
        }
        assert_eq!(HarnessResultKind::ALL.len(), 5);
    }

    #[test]
    fn op_from_u16_rejects_unknown() {
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command.op = 8;

        assert_eq!(
            command.decode_op(),
            Err(HarnessProtocolError::UnknownOp { raw: 8 })
        );
    }

    #[test]
    fn result_kind_from_u16_rejects_unknown() {
        let mut result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [0; 32]);
        result.kind = 5;

        assert_eq!(
            result.decode_kind(),
            Err(HarnessProtocolError::UnknownResultKind { raw: 5 })
        );
    }

    #[test]
    fn seq_mismatch_rejected() {
        let result = HarnessResultBlock::new(2, HarnessResultKind::Ok, [0; 32]);

        assert_eq!(
            result.validate_for_command(1),
            Err(HarnessProtocolError::SeqMismatch {
                command: 1,
                result: 2
            })
        );
    }

    #[test]
    fn serde_round_trip() {
        let command = HarnessCommandBlock::new(1, HarnessOp::GetState, [7; 32]);
        let encoded = serde_json::to_string(&command).expect("command serializes");
        let decoded: HarnessCommandBlock =
            serde_json::from_str(&encoded).expect("command deserializes");

        assert_eq!(decoded, command);

        let result = HarnessResultBlock::new(1, HarnessResultKind::Ok, [8; 32]);
        let encoded = serde_json::to_string(&result).expect("result serializes");
        let decoded: HarnessResultBlock =
            serde_json::from_str(&encoded).expect("result deserializes");

        assert_eq!(decoded, result);
    }

    #[test]
    fn doorbell_constants_distinct() {
        assert_ne!(doorbell::DOORBELL_RAISED, doorbell::DOORBELL_CLEAR);
        assert_eq!(doorbell::COMMAND_DOORBELL_OFFSET, 10);
        assert_eq!(doorbell::RESULT_READY_OFFSET, 10);
    }
}
