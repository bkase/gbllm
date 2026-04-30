//! Section placement types and bank/address helpers.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::section::SectionId;

pub const ROM_BANK_SIZE: u32 = 16 * 1024;
pub const ROM0_START: u16 = 0x0000;
pub const ROM0_END_EXCLUSIVE: u16 = 0x4000;
pub const ROMX_START: u16 = 0x4000;
pub const ROMX_END_EXCLUSIVE: u16 = 0x8000;

/// Physical or logical bank selected by layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BankIndex {
    Rom(u16),
    Sram(u8),
    Wram,
    Hram,
    Vram,
    Oam,
}

/// CPU-visible address space occupied by a placed section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressSpace {
    Rom0,
    RomX,
    Wram,
    Hram,
    Sram,
    Vram,
    Oam,
}

/// Final placement facts consumed by the encoder, listing, ROM builder, and
/// symbol writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedSection {
    pub id: SectionId,
    pub space: AddressSpace,
    pub bank: BankIndex,
    /// CPU-visible start address, not a ROM file offset.
    pub cpu_start: u16,
    pub final_size: u16,
    pub estimated_size: u16,
    /// Concrete padding chosen by layout for each `Align` item, keyed by the
    /// item's stable `seq_index`.
    pub alignment_padding: BTreeMap<u32, u16>,
}

impl PlacedSection {
    #[must_use]
    pub fn cpu_end_exclusive(&self) -> u32 {
        u32::from(self.cpu_start) + u32::from(self.final_size)
    }

    pub fn rom_file_offset(&self) -> Result<Option<usize>, LayoutError> {
        match (self.space, self.bank) {
            (AddressSpace::Rom0, BankIndex::Rom(0)) => {
                if self.cpu_start >= ROM0_END_EXCLUSIVE {
                    return Err(LayoutError::CpuAddressOutOfRange {
                        section_id: self.id,
                        space: self.space,
                        cpu_start: self.cpu_start,
                    });
                }
                Ok(Some(usize::from(self.cpu_start)))
            }
            (AddressSpace::RomX, BankIndex::Rom(bank)) if bank >= 1 => {
                if !(ROMX_START..ROMX_END_EXCLUSIVE).contains(&self.cpu_start) {
                    return Err(LayoutError::CpuAddressOutOfRange {
                        section_id: self.id,
                        space: self.space,
                        cpu_start: self.cpu_start,
                    });
                }
                Ok(Some(
                    usize::from(bank) * ROM_BANK_SIZE as usize
                        + usize::from(self.cpu_start - ROMX_START),
                ))
            }
            (
                AddressSpace::Wram
                | AddressSpace::Hram
                | AddressSpace::Sram
                | AddressSpace::Vram
                | AddressSpace::Oam,
                _,
            ) => Ok(None),
            _ => Err(LayoutError::BankSpaceMismatch {
                section_id: self.id,
                space: self.space,
                bank: self.bank,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutPlan {
    pub sections: Vec<PlacedSection>,
    pub bank_count: u16,
    pub free_bytes_per_bank: BTreeMap<BankIndex, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    CpuAddressOutOfRange {
        section_id: SectionId,
        space: AddressSpace,
        cpu_start: u16,
    },
    BankSpaceMismatch {
        section_id: SectionId,
        space: AddressSpace,
        bank: BankIndex,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuAddressOutOfRange {
                section_id,
                space,
                cpu_start,
            } => write!(
                f,
                "section {} starts at ${cpu_start:04X}, outside {space:?}",
                section_id.get()
            ),
            Self::BankSpaceMismatch {
                section_id,
                space,
                bank,
            } => write!(
                f,
                "section {} has address space {space:?} but bank {bank:?}",
                section_id.get()
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
#[test]
fn romx_file_offset_subtracts_4000() {
    let placed = PlacedSection {
        id: SectionId::new(1),
        space: AddressSpace::RomX,
        bank: BankIndex::Rom(3),
        cpu_start: 0x4123,
        final_size: 4,
        estimated_size: 4,
        alignment_padding: BTreeMap::new(),
    };

    assert_eq!(
        placed.rom_file_offset().expect("valid romx section"),
        Some(3 * 0x4000 + 0x0123)
    );
}
