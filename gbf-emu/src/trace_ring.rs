//! Canonical trace event normalization and bounded ring storage.

use std::collections::VecDeque;

use gbf_abi::trace::TraceEvent;
use gbf_hw::{mbc5, memory};
use serde::{Deserialize, Serialize};

use crate::primitives::{ClockCycles, EmuError};
use crate::trap::{BreakpointId, MemoryAccess, MemoryAccessKind, TrapKind};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NormalizedTraceEvent {
    MemoryWrite {
        addr: u16,
        value: u8,
        region: memory::MemoryRegion,
        bank: BankSnapshot,
        origin: TraceOrigin,
        cycle: ClockCycles,
    },
    RomBankSwitch {
        from: u16,
        to: u16,
        source: BankSwitchSource,
        cycle: ClockCycles,
    },
    SramBankSwitch {
        from: u8,
        to: u8,
        cycle: ClockCycles,
    },
    IoWrite {
        reg: u16,
        value: u8,
        cycle: ClockCycles,
    },
    TrapHit {
        trap_id: BreakpointId,
        kind: TrapKind,
        cycle: ClockCycles,
    },
    Typed(TraceEvent),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum BankSwitchSource {
    Bank1Write { value: u8 },
    Bank2Write { value: u8 },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TraceOrigin {
    GuestCpu,
    Dma,
    HostBus,
    HostPoke,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BankSnapshot {
    pub rom: u16,
    pub sram: u8,
    pub wramx: u8,
    pub vram: u8,
}

impl Default for BankSnapshot {
    fn default() -> Self {
        Self {
            rom: 1,
            sram: 0,
            wramx: 1,
            vram: 0,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndError,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TraceMapper {
    Fixed,
    Mbc5,
}

#[derive(Clone, Debug)]
pub struct TraceCursor {
    capacity: usize,
    drop_policy: TraceDropPolicy,
    events: VecDeque<NormalizedTraceEvent>,
    mapper: TraceMapper,
    bank1: u8,
    bank2: u8,
    sram: u8,
    wramx: u8,
    vram: u8,
}

impl TraceCursor {
    #[must_use]
    pub fn new(capacity: usize, drop_policy: TraceDropPolicy) -> Self {
        Self {
            capacity,
            drop_policy,
            events: VecDeque::with_capacity(capacity),
            mapper: TraceMapper::Mbc5,
            bank1: 1,
            bank2: 0,
            sram: 0,
            wramx: 1,
            vram: 0,
        }
    }

    #[must_use]
    pub fn with_mapper(capacity: usize, drop_policy: TraceDropPolicy, mapper: TraceMapper) -> Self {
        let mut cursor = Self::new(capacity, drop_policy);
        cursor.mapper = mapper;
        cursor
    }

    pub(crate) fn record(&mut self, event: NormalizedTraceEvent) -> Result<(), EmuError> {
        if self.capacity == 0 {
            return match self.drop_policy {
                TraceDropPolicy::DropOldest | TraceDropPolicy::DropNewest => Ok(()),
                TraceDropPolicy::HaltAndError => Err(EmuError::TraceCapacityExceeded {
                    capacity: self.capacity,
                }),
            };
        }

        if self.events.len() == self.capacity {
            match self.drop_policy {
                TraceDropPolicy::DropOldest => {
                    self.events.pop_front();
                }
                TraceDropPolicy::DropNewest => return Ok(()),
                TraceDropPolicy::HaltAndError => {
                    return Err(EmuError::TraceCapacityExceeded {
                        capacity: self.capacity,
                    });
                }
            }
        }
        self.events.push_back(event);
        Ok(())
    }

    pub(crate) fn record_access(
        &mut self,
        access: MemoryAccess,
        origin: TraceOrigin,
    ) -> Result<(), EmuError> {
        if access.kind != MemoryAccessKind::Write {
            return Ok(());
        }

        let bank = self.bank_snapshot();
        self.record(NormalizedTraceEvent::MemoryWrite {
            addr: access.addr,
            value: access.value,
            region: memory::classify(access.addr),
            bank,
            origin,
            cycle: access.cycle,
        })?;

        if memory::is_io(access.addr) {
            self.record(NormalizedTraceEvent::IoWrite {
                reg: access.addr,
                value: access.value,
                cycle: access.cycle,
            })?;
        }

        if self.mapper == TraceMapper::Mbc5 {
            match mbc5::classify_mbc_write_address(access.addr) {
                Some(mbc5::MbcRegisterClass::Bank1) => {
                    let from = self.current_rom_bank();
                    self.bank1 = access.value;
                    let to = self.current_rom_bank();
                    if from != to {
                        self.record(NormalizedTraceEvent::RomBankSwitch {
                            from,
                            to,
                            source: BankSwitchSource::Bank1Write {
                                value: access.value,
                            },
                            cycle: access.cycle,
                        })?;
                    }
                }
                Some(mbc5::MbcRegisterClass::Bank2) => {
                    let from = self.current_rom_bank();
                    self.bank2 = access.value & 0x01;
                    let to = self.current_rom_bank();
                    if from != to {
                        self.record(NormalizedTraceEvent::RomBankSwitch {
                            from,
                            to,
                            source: BankSwitchSource::Bank2Write {
                                value: access.value,
                            },
                            cycle: access.cycle,
                        })?;
                    }
                }
                Some(mbc5::MbcRegisterClass::Ramb) => {
                    let from = self.sram;
                    self.sram = access.value & 0x0F;
                    if from != self.sram {
                        self.record(NormalizedTraceEvent::SramBankSwitch {
                            from,
                            to: self.sram,
                            cycle: access.cycle,
                        })?;
                    }
                }
                Some(mbc5::MbcRegisterClass::Ramg | mbc5::MbcRegisterClass::Reserved) | None => {}
            }
        }

        Ok(())
    }

    pub(crate) fn record_trap_hit(
        &mut self,
        trap_id: BreakpointId,
        kind: TrapKind,
        cycle: ClockCycles,
    ) -> Result<(), EmuError> {
        self.record(NormalizedTraceEvent::TrapHit {
            trap_id,
            kind,
            cycle,
        })
    }

    pub(crate) fn record_typed(&mut self, event: TraceEvent) -> Result<(), EmuError> {
        self.record(NormalizedTraceEvent::Typed(event))
    }

    #[must_use]
    pub fn drain(&mut self) -> Vec<NormalizedTraceEvent> {
        self.events.drain(..).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn drop_policy(&self) -> TraceDropPolicy {
        self.drop_policy
    }

    #[must_use]
    pub const fn mapper(&self) -> TraceMapper {
        self.mapper
    }

    pub(crate) fn set_bank_snapshot(&mut self, bank: BankSnapshot) {
        self.bank1 = bank.rom as u8;
        self.bank2 = ((bank.rom >> 8) as u8) & 0x01;
        self.sram = bank.sram;
        self.wramx = bank.wramx;
        self.vram = bank.vram;
    }

    #[must_use]
    pub fn bank_snapshot(&self) -> BankSnapshot {
        BankSnapshot {
            rom: self.current_rom_bank(),
            sram: self.sram,
            wramx: self.wramx,
            vram: self.vram,
        }
    }

    #[must_use]
    pub const fn current_sram_bank(&self) -> u8 {
        self.sram
    }

    #[must_use]
    fn current_rom_bank(&self) -> u16 {
        mbc5::rom_bank_number(self.bank1, self.bank2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(addr: u16, value: u8, cycle: u64) -> MemoryAccess {
        MemoryAccess {
            addr,
            value,
            kind: MemoryAccessKind::Write,
            cycle: ClockCycles(cycle),
        }
    }

    #[test]
    fn memory_write_event_carries_bank_snapshot() {
        let mut cursor = TraceCursor::new(8, TraceDropPolicy::HaltAndError);

        cursor
            .record_access(write(0xC000, 0x55, 4), TraceOrigin::GuestCpu)
            .expect("record succeeds");

        assert_eq!(
            cursor.drain(),
            vec![NormalizedTraceEvent::MemoryWrite {
                addr: 0xC000,
                value: 0x55,
                region: memory::MemoryRegion::Wram0,
                bank: BankSnapshot::default(),
                origin: TraceOrigin::GuestCpu,
                cycle: ClockCycles(4),
            }]
        );
    }

    #[test]
    fn two_identical_consecutive_guest_writes_are_two_events() {
        let mut cursor = TraceCursor::new(8, TraceDropPolicy::HaltAndError);

        cursor
            .record_access(write(0xC000, 0x55, 4), TraceOrigin::GuestCpu)
            .unwrap();
        cursor
            .record_access(write(0xC000, 0x55, 8), TraceOrigin::GuestCpu)
            .unwrap();

        assert_eq!(cursor.drain().len(), 2);
    }

    #[test]
    fn rom_and_sram_bank_switch_events_are_derived() {
        let mut cursor = TraceCursor::new(16, TraceDropPolicy::HaltAndError);

        cursor
            .record_access(write(mbc5::MBC5_BANK1_BASE, 0x05, 4), TraceOrigin::GuestCpu)
            .unwrap();
        cursor
            .record_access(write(mbc5::MBC5_BANK2_BASE, 0x01, 8), TraceOrigin::GuestCpu)
            .unwrap();
        cursor
            .record_access(write(mbc5::MBC5_RAMB_BASE, 0x02, 12), TraceOrigin::GuestCpu)
            .unwrap();

        let events = cursor.drain();
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedTraceEvent::RomBankSwitch { from: 1, to: 5, .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedTraceEvent::RomBankSwitch {
                from: 5,
                to: 261,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedTraceEvent::SramBankSwitch { from: 0, to: 2, .. }
        )));
    }

    #[test]
    fn halt_and_error_on_overflow() {
        let mut cursor = TraceCursor::new(0, TraceDropPolicy::HaltAndError);
        assert_eq!(
            cursor.record_access(write(0xC000, 1, 4), TraceOrigin::GuestCpu),
            Err(EmuError::TraceCapacityExceeded { capacity: 0 })
        );
    }

    #[test]
    fn drop_oldest_under_pressure() {
        let mut cursor = TraceCursor::new(1, TraceDropPolicy::DropOldest);
        cursor
            .record_access(write(0xC000, 1, 4), TraceOrigin::GuestCpu)
            .unwrap();
        cursor
            .record_access(write(0xC001, 2, 8), TraceOrigin::GuestCpu)
            .unwrap();

        let events = cursor.drain();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            NormalizedTraceEvent::MemoryWrite { addr: 0xC001, .. }
        ));
    }
}
