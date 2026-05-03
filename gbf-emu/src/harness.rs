//! Host-side plumbing for F-A3 harness command/result blocks.

use gbf_abi::harness::{HarnessCommandBlock, HarnessProtocolError, HarnessResultBlock, doorbell};
use gbf_hw::memory;
use serde::{Deserialize, Serialize};

use crate::primitives::EmuError;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct HarnessSlot {
    pub sram_bank: u8,
    pub command_addr: u16,
    pub result_addr: u16,
    pub doorbell_addr: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessCommand {
    pub block: HarnessCommandBlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessResult {
    pub block: HarnessResultBlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessChannel {
    slot: HarnessSlot,
    last_seen_seq: u32,
    has_seen_command: bool,
}

impl HarnessChannel {
    #[must_use]
    pub const fn new(slot: HarnessSlot) -> Self {
        Self {
            slot,
            last_seen_seq: 0,
            has_seen_command: false,
        }
    }

    #[must_use]
    pub const fn slot(&self) -> HarnessSlot {
        self.slot
    }

    #[must_use]
    pub const fn last_seen_seq(&self) -> u32 {
        self.last_seen_seq
    }

    pub(crate) fn read_command_from<M: HarnessMemory>(
        &mut self,
        mem: &M,
    ) -> Result<Option<HarnessCommand>, EmuError> {
        let doorbell = mem.read_sram_bank(self.slot.sram_bank, self.slot.doorbell_addr)?;
        if doorbell != doorbell::DOORBELL_RAISED {
            return Ok(None);
        }

        let bytes = mem.read_sram_bank_range(
            self.slot.sram_bank,
            self.slot.command_addr,
            HarnessCommandBlock::SIZE,
        )?;
        let block = command_from_bytes(&bytes)?;

        if self.has_seen_command && block.seq == self.last_seen_seq {
            return Ok(None);
        }

        let expected = if self.has_seen_command {
            self.last_seen_seq.wrapping_add(1)
        } else {
            1
        };
        if block.seq != expected {
            return Err(EmuError::HarnessSequenceMismatch {
                observed: block.seq,
                expected,
            });
        }

        self.last_seen_seq = block.seq;
        self.has_seen_command = true;
        Ok(Some(HarnessCommand { block }))
    }

    pub(crate) fn write_result_to<M: HarnessMemory>(
        &mut self,
        mem: &mut M,
        result: HarnessResult,
    ) -> Result<Vec<(u16, u8)>, EmuError> {
        let mut block = result.block;
        if !self.has_seen_command {
            return Err(EmuError::HarnessSequenceMismatch {
                observed: block.seq,
                expected: 0,
            });
        }
        block
            .validate_for_command(self.last_seen_seq)
            .map_err(protocol_error_to_emu)?;
        block.mark_ready();
        let mut writes = Vec::with_capacity(HarnessResultBlock::SIZE + 1);
        for (offset, value) in block.to_bytes().iter().copied().enumerate() {
            let addr = checked_addr(self.slot.result_addr, offset)?;
            mem.write_sram_bank(self.slot.sram_bank, addr, value)?;
            writes.push((addr, value));
        }
        mem.write_sram_bank(
            self.slot.sram_bank,
            self.slot.doorbell_addr,
            doorbell::DOORBELL_RAISED,
        )?;
        writes.push((self.slot.doorbell_addr, doorbell::DOORBELL_RAISED));
        Ok(writes)
    }
}

pub(crate) trait HarnessMemory {
    fn read_sram_bank(&self, bank: u8, addr: u16) -> Result<u8, EmuError>;
    fn read_sram_bank_range(&self, bank: u8, addr: u16, len: usize) -> Result<Vec<u8>, EmuError> {
        (0..len)
            .map(|offset| self.read_sram_bank(bank, checked_addr(addr, offset)?))
            .collect()
    }
    fn write_sram_bank(&mut self, bank: u8, addr: u16, value: u8) -> Result<(), EmuError>;
}

fn checked_addr(addr: u16, offset: usize) -> Result<u16, EmuError> {
    let offset = u16::try_from(offset).map_err(|_| EmuError::MemoryAccess {
        addr,
        reason: format!("offset {offset} exceeds u16 address space"),
    })?;
    addr.checked_add(offset)
        .ok_or_else(|| EmuError::MemoryAccess {
            addr,
            reason: "harness block address overflows u16 address space".to_owned(),
        })
}

fn command_from_bytes(bytes: &[u8]) -> Result<HarnessCommandBlock, EmuError> {
    let bytes: &[u8; HarnessCommandBlock::SIZE] =
        bytes.try_into().map_err(|_| EmuError::MemoryAccess {
            addr: 0,
            reason: format!(
                "expected {} harness command bytes",
                HarnessCommandBlock::SIZE
            ),
        })?;
    HarnessCommandBlock::from_bytes(bytes).map_err(protocol_error_to_emu)
}

fn protocol_error_to_emu(error: HarnessProtocolError) -> EmuError {
    match error {
        HarnessProtocolError::BadMagic { observed, expected } => {
            EmuError::HarnessMagicMismatch { observed, expected }
        }
        HarnessProtocolError::SeqMismatch { command, result } => {
            EmuError::HarnessSequenceMismatch {
                observed: result,
                expected: command,
            }
        }
        other => EmuError::MemoryAccess {
            addr: 0,
            reason: other.to_string(),
        },
    }
}

pub(crate) fn sram_offset(
    bank: u8,
    addr: u16,
    len: usize,
    ram_len: usize,
) -> Result<usize, EmuError> {
    if !memory::is_sram_window(addr) {
        return Err(EmuError::MemoryAccess {
            addr,
            reason: "harness slot address is outside the cartridge SRAM window".to_owned(),
        });
    }
    let window_offset = (addr - memory::SRAM_BASE) as usize;
    let window_end = window_offset
        .checked_add(len)
        .ok_or(EmuError::HarnessSramOutOfRange {
            bank,
            addr,
            len,
            ram_len,
        })?;
    if window_end > memory::SRAM_BANK_SIZE_BYTES as usize {
        return Err(EmuError::HarnessSramOutOfRange {
            bank,
            addr,
            len,
            ram_len,
        });
    }
    let offset = bank as usize * memory::SRAM_BANK_SIZE_BYTES as usize + window_offset;
    let end = offset
        .checked_add(len)
        .ok_or(EmuError::HarnessSramOutOfRange {
            bank,
            addr,
            len,
            ram_len,
        })?;
    if end > ram_len {
        return Err(EmuError::HarnessSramOutOfRange {
            bank,
            addr,
            len,
            ram_len,
        });
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_abi::harness::{HarnessOp, HarnessResultKind};

    #[derive(Default)]
    struct FakeMemory {
        ram: Vec<u8>,
    }

    impl HarnessMemory for FakeMemory {
        fn read_sram_bank(&self, bank: u8, addr: u16) -> Result<u8, EmuError> {
            Ok(self.ram[sram_offset(bank, addr, 1, self.ram.len())?])
        }

        fn write_sram_bank(&mut self, bank: u8, addr: u16, value: u8) -> Result<(), EmuError> {
            let offset = sram_offset(bank, addr, 1, self.ram.len())?;
            self.ram[offset] = value;
            Ok(())
        }
    }

    fn slot() -> HarnessSlot {
        HarnessSlot {
            sram_bank: 0,
            command_addr: 0xA000,
            result_addr: 0xA040,
            doorbell_addr: 0xA080,
        }
    }

    #[test]
    fn poll_returns_none_when_no_doorbell() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());

        assert_eq!(channel.read_command_from(&mut mem), Ok(None));
    }

    #[test]
    fn poll_parses_f_a3_command_block() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [7; 32]);
        command.raise_doorbell();
        for (offset, value) in command.to_bytes().iter().copied().enumerate() {
            mem.write_sram_bank(0, checked_addr(0xA000, offset).unwrap(), value)
                .unwrap();
        }
        mem.write_sram_bank(0, 0xA080, doorbell::DOORBELL_RAISED)
            .unwrap();

        let parsed = channel
            .read_command_from(&mem)
            .expect("poll succeeds")
            .expect("command present");

        assert_eq!(parsed.block, command);
        assert_eq!(channel.last_seen_seq(), 1);
    }

    #[test]
    fn poll_rejects_seq_mismatch_on_command_block_seq() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());
        let mut command = HarnessCommandBlock::new(2, HarnessOp::StepSlice, [0; 32]);
        command.raise_doorbell();
        for (offset, value) in command.to_bytes().iter().copied().enumerate() {
            mem.write_sram_bank(0, checked_addr(0xA000, offset).unwrap(), value)
                .unwrap();
        }
        mem.write_sram_bank(0, 0xA080, doorbell::DOORBELL_RAISED)
            .unwrap();

        assert_eq!(
            channel.read_command_from(&mem),
            Err(EmuError::HarnessSequenceMismatch {
                observed: 2,
                expected: 1
            })
        );
        assert_eq!(channel.last_seen_seq(), 0);
    }

    #[test]
    fn write_result_writes_result_block_bytes() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());
        channel.last_seen_seq = 1;
        channel.has_seen_command = true;
        let result = HarnessResult {
            block: HarnessResultBlock::new(1, HarnessResultKind::Ok, [3; 32]),
        };

        channel.write_result_to(&mut mem, result).unwrap();

        assert_eq!(mem.read_sram_bank(0, 0xA040).unwrap(), b'H');
        assert_eq!(
            mem.read_sram_bank(0, 0xA040 + doorbell::RESULT_READY_OFFSET as u16)
                .unwrap(),
            doorbell::DOORBELL_RAISED
        );
        assert_eq!(
            mem.read_sram_bank(0, 0xA080).unwrap(),
            doorbell::DOORBELL_RAISED
        );
    }

    #[test]
    fn repeated_raised_doorbell_with_seen_seq_is_not_a_new_edge() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());
        let mut command = HarnessCommandBlock::new(1, HarnessOp::StepSlice, [0; 32]);
        command.raise_doorbell();
        for (offset, value) in command.to_bytes().iter().copied().enumerate() {
            mem.write_sram_bank(0, checked_addr(0xA000, offset).unwrap(), value)
                .unwrap();
        }
        mem.write_sram_bank(0, 0xA080, doorbell::DOORBELL_RAISED)
            .unwrap();

        assert!(channel.read_command_from(&mem).unwrap().is_some());
        assert_eq!(channel.read_command_from(&mem), Ok(None));
    }

    #[test]
    fn write_result_validates_magic_and_sequence_before_ready() {
        let mut mem = FakeMemory {
            ram: vec![0; 0x2000],
        };
        let mut channel = HarnessChannel::new(slot());
        channel.last_seen_seq = 7;
        channel.has_seen_command = true;

        let bad_seq = HarnessResult {
            block: HarnessResultBlock::new(8, HarnessResultKind::Ok, [0; 32]),
        };
        assert_eq!(
            channel.write_result_to(&mut mem, bad_seq),
            Err(EmuError::HarnessSequenceMismatch {
                observed: 8,
                expected: 7
            })
        );

        let mut bad_magic = HarnessResultBlock::new(7, HarnessResultKind::Ok, [0; 32]);
        bad_magic.magic = *b"BAD!";
        assert!(matches!(
            channel.write_result_to(&mut mem, HarnessResult { block: bad_magic }),
            Err(EmuError::HarnessMagicMismatch { .. })
        ));
        assert_eq!(mem.read_sram_bank(0, 0xA080), Ok(0));
    }

    #[test]
    fn sram_range_cannot_cross_bank_window() {
        assert!(matches!(
            sram_offset(0, 0xBFFF, 2, 0x4000),
            Err(EmuError::HarnessSramOutOfRange { .. })
        ));
    }
}
