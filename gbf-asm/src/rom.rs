//! Deterministic MBC5 ROM assembly and cartridge-header construction.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::encoder::{EncodeError, EncodedSection, encode_section};
use crate::isa::Instr;
use crate::layout::{
    AddressSpace, BankIndex, LayoutError, LayoutPlan, PlacedSection,
    ROM_BANK_SIZE as LAYOUT_ROM_BANK_SIZE, ROM0_END_EXCLUSIVE, ROMX_END_EXCLUSIVE,
};
use crate::provenance::{InstrProvenance, PlanningStage};
use crate::section::{
    DataBlock, Label, LegalizedSection, OrderedItem, SectionId, SectionPrivilege, SectionRole,
    SymbolId,
};
use crate::symbols::SymbolName;

pub const HEADER_START: usize = 0x0100;
pub const HEADER_END_EXCLUSIVE: usize = 0x0150;
pub const ENTRY_POINT: u16 = 0x0150;
pub const ROM_BANK_SIZE: usize = LAYOUT_ROM_BANK_SIZE as usize;
const HEADER_SECTION_ID: SectionId = SectionId::new(0xFFFF_FFFE);

// TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is
// populated.
pub const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartridgeHeader {
    pub title: String,
    pub mbc_type: MbcType,
    pub rom_size: RomSize,
    pub ram_size: RamSize,
    pub destination_code: DestinationCode,
    pub new_licensee_code: [u8; 2],
    pub mask_rom_version: u8,
}

impl CartridgeHeader {
    pub fn new(title: impl Into<String>) -> Result<Self, RomAssemblyError> {
        let header = Self {
            title: title.into(),
            ..Self::default()
        };
        header.validate()?;
        Ok(header)
    }

    pub fn validate(&self) -> Result<(), RomAssemblyError> {
        let title = self.title.as_bytes();
        if title.len() > 11 {
            return Err(RomAssemblyError::InvalidTitle {
                reason: "title must be at most 11 ASCII bytes",
            });
        }
        if !title.is_ascii() {
            return Err(RomAssemblyError::InvalidTitle {
                reason: "title must be ASCII",
            });
        }
        if title.contains(&0) {
            return Err(RomAssemblyError::InvalidTitle {
                reason: "title must not contain interior NUL bytes",
            });
        }
        if !self.new_licensee_code.iter().all(u8::is_ascii_alphanumeric) {
            return Err(RomAssemblyError::InvalidLicenseeCode {
                code: self.new_licensee_code,
            });
        }
        Ok(())
    }
}

impl Default for CartridgeHeader {
    fn default() -> Self {
        Self {
            title: "GBFASM".to_owned(),
            mbc_type: MbcType::Mbc5,
            rom_size: RomSize::Kib32,
            ram_size: RamSize::None,
            destination_code: DestinationCode::Overseas,
            new_licensee_code: *b"00",
            mask_rom_version: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbcType {
    Mbc5,
    Mbc5Ram,
    Mbc5RamBattery,
}

impl MbcType {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Mbc5 => 0x19,
            Self::Mbc5Ram => 0x1A,
            Self::Mbc5RamBattery => 0x1B,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomSize {
    Kib32,
    Kib64,
    Kib128,
    Kib256,
    Kib512,
    Mib1,
    Mib2,
    Mib4,
    Mib8,
}

impl RomSize {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Kib32 => 0x00,
            Self::Kib64 => 0x01,
            Self::Kib128 => 0x02,
            Self::Kib256 => 0x03,
            Self::Kib512 => 0x04,
            Self::Mib1 => 0x05,
            Self::Mib2 => 0x06,
            Self::Mib4 => 0x07,
            Self::Mib8 => 0x08,
        }
    }

    #[must_use]
    pub const fn bank_count(self) -> u16 {
        match self {
            Self::Kib32 => 2,
            Self::Kib64 => 4,
            Self::Kib128 => 8,
            Self::Kib256 => 16,
            Self::Kib512 => 32,
            Self::Mib1 => 64,
            Self::Mib2 => 128,
            Self::Mib4 => 256,
            Self::Mib8 => 512,
        }
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bank_count() as usize * ROM_BANK_SIZE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RamSize {
    None,
    Kib8,
    Kib32,
    Kib64,
    Kib128,
}

impl RamSize {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::None => 0x00,
            Self::Kib8 => 0x02,
            Self::Kib32 => 0x03,
            Self::Kib128 => 0x04,
            Self::Kib64 => 0x05,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCode {
    Japan,
    Overseas,
}

impl DestinationCode {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Japan => 0x00,
            Self::Overseas => 0x01,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomAssemblyError {
    UserHeaderSectionRejected {
        id: SectionId,
    },
    HeaderRangeCollision {
        id: SectionId,
        start: usize,
        end_exclusive: usize,
    },
    SectionCollision {
        id: SectionId,
        other_id: SectionId,
        start: usize,
        end_exclusive: usize,
    },
    SectionPlacementMismatch {
        section_id: SectionId,
        placed_id: SectionId,
    },
    SectionSizeMismatch {
        id: SectionId,
        expected: u16,
        actual: usize,
    },
    SectionExceedsBankBoundary {
        id: SectionId,
        bank: BankIndex,
        cpu_start: u16,
        len: u32,
        end_exclusive: u32,
        bank_end_exclusive: u32,
    },
    BankIndexOutOfRange {
        id: SectionId,
        bank: BankIndex,
        max_valid_bank: u16,
    },
    InvalidTitle {
        reason: &'static str,
    },
    InvalidLicenseeCode {
        code: [u8; 2],
    },
    InvalidRomSizeForLayout {
        requested_banks: u16,
        header_banks: u16,
    },
    MissingEntryPoint {
        addr: u16,
    },
    NonRomSection {
        id: SectionId,
        space: AddressSpace,
    },
    Layout(LayoutError),
    Encode(EncodeError),
}

impl fmt::Display for RomAssemblyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserHeaderSectionRejected { id } => {
                write!(
                    f,
                    "user section {} uses internal HeaderCartridge role",
                    id.get()
                )
            }
            Self::HeaderRangeCollision {
                id,
                start,
                end_exclusive,
            } => write!(
                f,
                "section {} overlaps cartridge header bytes ${start:04X}..${end_exclusive:04X}",
                id.get()
            ),
            Self::SectionCollision {
                id,
                other_id,
                start,
                end_exclusive,
            } => write!(
                f,
                "section {} overlaps section {} at ROM bytes ${start:04X}..${end_exclusive:04X}",
                id.get(),
                other_id.get()
            ),
            Self::SectionPlacementMismatch {
                section_id,
                placed_id,
            } => write!(
                f,
                "encoded section {} was paired with placement for section {}",
                section_id.get(),
                placed_id.get()
            ),
            Self::SectionSizeMismatch {
                id,
                expected,
                actual,
            } => write!(
                f,
                "encoded section {} has {actual} bytes but placement declares {expected}",
                id.get()
            ),
            Self::SectionExceedsBankBoundary {
                id,
                bank,
                cpu_start,
                len,
                end_exclusive,
                bank_end_exclusive,
            } => write!(
                f,
                "section {} in {bank} starts at ${cpu_start:04X}, len {len}, ends at ${end_exclusive:04X}, beyond ${bank_end_exclusive:04X}",
                id.get()
            ),
            Self::BankIndexOutOfRange {
                id,
                bank,
                max_valid_bank,
            } => write!(
                f,
                "section {} uses {bank}, beyond maximum ROM bank {max_valid_bank}",
                id.get()
            ),
            Self::InvalidTitle { reason } => write!(f, "invalid cartridge title: {reason}"),
            Self::InvalidLicenseeCode { code } => {
                write!(f, "invalid new licensee code {code:?}")
            }
            Self::InvalidRomSizeForLayout {
                requested_banks,
                header_banks,
            } => write!(
                f,
                "layout needs {requested_banks} banks but header declares {header_banks}"
            ),
            Self::MissingEntryPoint { addr } => {
                write!(
                    f,
                    "ROM has no encoded section at cartridge entry point ${addr:04X}"
                )
            }
            Self::NonRomSection { id, space } => {
                write!(f, "section {} is in {space:?}, not ROM", id.get())
            }
            Self::Layout(error) => write!(f, "{error}"),
            Self::Encode(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RomAssemblyError {}

impl From<LayoutError> for RomAssemblyError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

impl From<EncodeError> for RomAssemblyError {
    fn from(value: EncodeError) -> Self {
        Self::Encode(value)
    }
}

pub fn assemble_rom(
    encoded: &[(EncodedSection, PlacedSection)],
    layout: &LayoutPlan,
    header: &CartridgeHeader,
) -> Result<Vec<u8>, RomAssemblyError> {
    header.validate()?;
    let header_banks = header.rom_size.bank_count();
    if layout.bank_count > header_banks {
        return Err(RomAssemblyError::InvalidRomSizeForLayout {
            requested_banks: layout.bank_count,
            header_banks,
        });
    }

    let mut rom = vec![0xFF; header.rom_size.bytes()];
    let mut occupied = vec![None; rom.len()];
    for (section, placed) in encoded {
        copy_section(&mut rom, &mut occupied, section, placed, header_banks)?;
    }
    if occupied[usize::from(ENTRY_POINT)].is_none() {
        return Err(RomAssemblyError::MissingEntryPoint { addr: ENTRY_POINT });
    }

    let header_section = build_header_section(header)?;
    let header_placement = header_placement();
    let encoded_header = encode_section(&header_section, &header_placement)?;
    let header_start = header_placement
        .rom_file_offset()?
        .expect("header is in ROM0");
    rom[header_start..header_start + encoded_header.bytes.len()]
        .copy_from_slice(&encoded_header.bytes);

    let global = global_checksum(&rom);
    rom[0x014E] = (global >> 8) as u8;
    rom[0x014F] = (global & 0x00FF) as u8;
    Ok(rom)
}

fn copy_section(
    rom: &mut [u8],
    occupied: &mut [Option<SectionId>],
    encoded: &EncodedSection,
    placed: &PlacedSection,
    header_banks: u16,
) -> Result<(), RomAssemblyError> {
    if encoded.id == HEADER_SECTION_ID {
        return Err(RomAssemblyError::UserHeaderSectionRejected { id: encoded.id });
    }
    if encoded.id != placed.id {
        return Err(RomAssemblyError::SectionPlacementMismatch {
            section_id: encoded.id,
            placed_id: placed.id,
        });
    }
    if encoded.bytes.len() != usize::from(placed.final_size) {
        return Err(RomAssemblyError::SectionSizeMismatch {
            id: encoded.id,
            expected: placed.final_size,
            actual: encoded.bytes.len(),
        });
    }
    let bank = match placed.bank {
        BankIndex::Rom(bank) => bank,
        _ => {
            return Err(RomAssemblyError::NonRomSection {
                id: encoded.id,
                space: placed.space,
            });
        }
    };
    if bank >= header_banks {
        return Err(RomAssemblyError::BankIndexOutOfRange {
            id: encoded.id,
            bank: placed.bank,
            max_valid_bank: header_banks - 1,
        });
    }
    let Some(offset) = placed.rom_file_offset()? else {
        return Err(RomAssemblyError::NonRomSection {
            id: encoded.id,
            space: placed.space,
        });
    };
    let end = offset + encoded.bytes.len();
    if overlaps_header(offset, end) {
        return Err(RomAssemblyError::HeaderRangeCollision {
            id: encoded.id,
            start: offset,
            end_exclusive: end,
        });
    }
    if let Some(other_id) = occupied[offset..end].iter().find_map(|owner| *owner) {
        return Err(RomAssemblyError::SectionCollision {
            id: encoded.id,
            other_id,
            start: offset,
            end_exclusive: end,
        });
    }
    let bank_end = (usize::from(bank) + 1) * ROM_BANK_SIZE;
    if end > bank_end {
        return Err(RomAssemblyError::SectionExceedsBankBoundary {
            id: encoded.id,
            bank: placed.bank,
            cpu_start: placed.cpu_start,
            len: encoded.bytes.len() as u32,
            end_exclusive: placed.cpu_end_exclusive(),
            bank_end_exclusive: match placed.space {
                AddressSpace::Rom0 => u32::from(ROM0_END_EXCLUSIVE),
                AddressSpace::RomX => u32::from(ROMX_END_EXCLUSIVE),
                _ => unreachable!("non-ROM placements are rejected earlier"),
            },
        });
    }
    rom[offset..end].copy_from_slice(&encoded.bytes);
    occupied[offset..end].fill(Some(encoded.id));
    Ok(())
}

fn overlaps_header(start: usize, end_exclusive: usize) -> bool {
    start < HEADER_END_EXCLUSIVE && end_exclusive > HEADER_START
}

fn header_placement() -> PlacedSection {
    PlacedSection {
        id: HEADER_SECTION_ID,
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start: HEADER_START as u16,
        final_size: (HEADER_END_EXCLUSIVE - HEADER_START) as u16,
        estimated_size: (HEADER_END_EXCLUSIVE - HEADER_START) as u16,
        alignment_padding: BTreeMap::new(),
    }
}

fn build_header_section(header: &CartridgeHeader) -> Result<LegalizedSection, RomAssemblyError> {
    header.validate()?;
    let provenance =
        InstrProvenance::new(PlanningStage::Backend).with_source_op("rom.cartridge_header");
    let header_fields = header_fields(header);
    let checksum = header_checksum_bytes(&header_fields);
    let name = SymbolName::runtime("cartridge", "header").expect("valid static symbol");

    Ok(LegalizedSection {
        id: HEADER_SECTION_ID,
        role: SectionRole::HeaderCartridge,
        name: name.clone(),
        privilege: SectionPrivilege::normal(),
        align: NonZeroU16::new(1).expect("1 is nonzero"),
        size_hint_bytes: Some((HEADER_END_EXCLUSIVE - HEADER_START) as u32),
        next_seq_index: 6,
        labels: vec![OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name,
            },
            0,
            provenance.clone(),
        )],
        instrs: vec![
            OrderedItem::new(Instr::Nop, 1, provenance.clone()),
            OrderedItem::new(
                Instr::JpAbs {
                    cond: None,
                    addr: ENTRY_POINT,
                },
                2,
                provenance.clone(),
            ),
        ],
        data_blocks: vec![
            OrderedItem::new(
                DataBlock::Bytes(NINTENDO_LOGO.to_vec()),
                3,
                provenance.clone(),
            ),
            OrderedItem::new(DataBlock::Bytes(header_fields), 4, provenance.clone()),
            OrderedItem::new(DataBlock::Bytes(vec![checksum, 0x00, 0x00]), 5, provenance),
        ],
        alignments: vec![],
    })
}

fn header_fields(header: &CartridgeHeader) -> Vec<u8> {
    let mut fields = Vec::with_capacity(25);
    let mut title = [0_u8; 11];
    title[..header.title.len()].copy_from_slice(header.title.as_bytes());
    fields.extend_from_slice(&title);
    fields.extend_from_slice(b"0000");
    fields.push(0x00);
    fields.extend_from_slice(&header.new_licensee_code);
    fields.push(0x00);
    fields.push(header.mbc_type.header_byte());
    fields.push(header.rom_size.header_byte());
    fields.push(header.ram_size.header_byte());
    fields.push(header.destination_code.header_byte());
    fields.push(0x33);
    fields.push(header.mask_rom_version);
    fields
}

#[must_use]
pub fn header_checksum(rom: &[u8]) -> u8 {
    header_checksum_bytes(&rom[0x0134..=0x014C])
}

fn header_checksum_bytes(bytes: &[u8]) -> u8 {
    let mut x = 0_u8;
    for byte in bytes {
        x = x.wrapping_sub(*byte).wrapping_sub(1);
    }
    x
}

#[must_use]
pub fn global_checksum(rom: &[u8]) -> u16 {
    let mut sum = 0_u16;
    for (idx, byte) in rom.iter().enumerate() {
        if idx == 0x014E || idx == 0x014F {
            continue;
        }
        sum = sum.wrapping_add(u16::from(*byte));
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::EncodedSection;
    use crate::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};

    fn layout(bank_count: u16) -> LayoutPlan {
        LayoutPlan {
            sections: Vec::new(),
            bank_count,
            free_bytes_per_bank: BTreeMap::new(),
            reserved_ranges: Vec::new(),
        }
    }

    fn encoded_section(
        id: u32,
        bank: u16,
        cpu_start: u16,
        bytes: &[u8],
    ) -> (EncodedSection, PlacedSection) {
        let space = if bank == 0 {
            AddressSpace::Rom0
        } else {
            AddressSpace::RomX
        };
        (
            EncodedSection {
                id: SectionId::new(id),
                bytes: bytes.to_vec(),
                item_spans: Vec::new(),
            },
            PlacedSection {
                id: SectionId::new(id),
                space,
                bank: BankIndex::Rom(bank),
                cpu_start,
                final_size: bytes.len() as u16,
                estimated_size: bytes.len() as u16,
                alignment_padding: BTreeMap::new(),
            },
        )
    }

    fn entry_section() -> (EncodedSection, PlacedSection) {
        encoded_section(1, 0, ENTRY_POINT, &[0x00])
    }

    fn with_entry(
        sections: impl IntoIterator<Item = (EncodedSection, PlacedSection)>,
    ) -> Vec<(EncodedSection, PlacedSection)> {
        std::iter::once(entry_section()).chain(sections).collect()
    }

    fn assemble(header: CartridgeHeader) -> Vec<u8> {
        let sections = [entry_section()];
        assemble_rom(&sections, &layout(2), &header).expect("assemble rom")
    }

    #[test]
    fn header_checksum_known_vector() {
        let rom = assemble(CartridgeHeader::default());
        assert_eq!(rom[0x014D], header_checksum(&rom));
        let expected = header_checksum_bytes(&header_fields(&CartridgeHeader::default()));
        assert_eq!(rom[0x014D], expected);
    }

    #[test]
    fn global_checksum_round_trip() {
        let rom = assemble(CartridgeHeader::default());
        let expected = global_checksum(&rom);
        let stored = u16::from_be_bytes([rom[0x014E], rom[0x014F]]);
        assert_eq!(stored, expected);
    }

    #[test]
    fn power_of_two_size() {
        for rom_size in [
            RomSize::Kib32,
            RomSize::Kib64,
            RomSize::Kib128,
            RomSize::Kib256,
            RomSize::Kib512,
            RomSize::Mib1,
            RomSize::Mib2,
            RomSize::Mib4,
            RomSize::Mib8,
        ] {
            let header = CartridgeHeader {
                rom_size,
                ..CartridgeHeader::default()
            };
            let sections = [entry_section()];
            let rom = assemble_rom(&sections, &layout(2), &header).expect("assemble rom");
            assert!(rom.len().is_power_of_two());
            assert!(rom.len() >= 32 * 1024);
        }
    }

    #[test]
    fn nintendo_logo_present() {
        let rom = assemble(CartridgeHeader::default());
        assert_eq!(&rom[0x0104..0x0134], &NINTENDO_LOGO);
    }

    #[test]
    fn ram_size_header_bytes() {
        assert_eq!(RamSize::Kib64.header_byte(), 0x05);
        assert_eq!(RamSize::Kib128.header_byte(), 0x04);
    }

    #[test]
    fn bank_n_at_correct_offset() {
        let section = encoded_section(7, 3, 0x4000, &[0xAA, 0xBB, 0xCC]);
        let header = CartridgeHeader {
            rom_size: RomSize::Kib64,
            ..CartridgeHeader::default()
        };
        let sections = with_entry([section]);
        let rom = assemble_rom(&sections, &layout(4), &header).expect("assemble rom");
        assert_eq!(
            &rom[3 * ROM_BANK_SIZE..3 * ROM_BANK_SIZE + 3],
            &[0xAA, 0xBB, 0xCC]
        );
    }

    #[test]
    fn unused_regions_are_ff() {
        let section = encoded_section(7, 1, 0x4000, &[0xAA]);
        let sections = with_entry([section]);
        let rom =
            assemble_rom(&sections, &layout(2), &CartridgeHeader::default()).expect("assemble rom");
        assert_eq!(rom[ROM_BANK_SIZE + 1], 0xFF);
        assert_eq!(rom[0x0151], 0xFF);
    }

    #[test]
    fn deterministic() {
        assert_eq!(
            assemble(CartridgeHeader::default()),
            assemble(CartridgeHeader::default())
        );
    }

    #[test]
    fn invalid_title_rejected() {
        let err = CartridgeHeader::new("title_too_long").expect_err("invalid title");
        assert!(matches!(err, RomAssemblyError::InvalidTitle { .. }));
        let err = CartridgeHeader {
            title: "BAD\0".to_owned(),
            ..CartridgeHeader::default()
        }
        .validate()
        .expect_err("invalid title");
        assert!(matches!(err, RomAssemblyError::InvalidTitle { .. }));
    }

    #[test]
    fn user_header_range_rejected() {
        let section = encoded_section(1, 0, 0x0100, &[0x00]);
        let sections = with_entry([section]);
        let err = assemble_rom(&sections, &layout(2), &CartridgeHeader::default())
            .expect_err("header collision");
        assert!(matches!(err, RomAssemblyError::HeaderRangeCollision { .. }));
    }

    #[test]
    fn overlapping_sections_are_rejected() {
        let first = encoded_section(2, 1, 0x4000, &[0xAA, 0xBB]);
        let second = encoded_section(3, 1, 0x4001, &[0xCC]);
        let sections = with_entry([first, second]);
        let err = assemble_rom(&sections, &layout(2), &CartridgeHeader::default())
            .expect_err("section collision");
        assert!(matches!(err, RomAssemblyError::SectionCollision { .. }));
    }

    #[test]
    fn section_size_mismatch_is_rejected() {
        let (mut encoded, placed) = encoded_section(2, 1, 0x4000, &[0xAA, 0xBB]);
        encoded.bytes.pop();
        let sections = with_entry([(encoded, placed)]);
        let err = assemble_rom(&sections, &layout(2), &CartridgeHeader::default())
            .expect_err("section size mismatch");
        assert!(matches!(err, RomAssemblyError::SectionSizeMismatch { .. }));

        let (mut encoded, placed) = encoded_section(3, 1, 0x4004, &[0xCC]);
        encoded.bytes.push(0xDD);
        let sections = with_entry([(encoded, placed)]);
        let err = assemble_rom(&sections, &layout(2), &CartridgeHeader::default())
            .expect_err("section size mismatch");
        assert!(matches!(err, RomAssemblyError::SectionSizeMismatch { .. }));
    }

    #[test]
    fn entry_point_is_required() {
        let err = assemble_rom(&[], &layout(2), &CartridgeHeader::default())
            .expect_err("missing entry point");
        assert_eq!(
            err,
            RomAssemblyError::MissingEntryPoint { addr: ENTRY_POINT }
        );
    }

    #[test]
    fn invalid_rom_size_for_layout() {
        let err = assemble_rom(&[], &layout(4), &CartridgeHeader::default())
            .expect_err("too small header rom size");
        assert_eq!(
            err,
            RomAssemblyError::InvalidRomSizeForLayout {
                requested_banks: 4,
                header_banks: 2,
            }
        );
    }

    #[test]
    fn public_enums_reject_unknown_serde_values() {
        assert!(serde_json::from_str::<RomSize>(r#""kib16""#).is_err());
        assert!(serde_json::from_str::<RamSize>(r#""kib16""#).is_err());
        assert!(serde_json::from_str::<MbcType>(r#""mbc1""#).is_err());
    }
}
