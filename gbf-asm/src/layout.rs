//! Section placement types and bank/address helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::section::{
    BranchKind, DataBlock, ItemOrder, LegalizationOp, LoweredSection, SectionId, SectionRole,
};

pub const ROM_BANK_SIZE: u32 = 16 * 1024;
pub const ROM0_START: u16 = 0x0000;
pub const ROM0_END_EXCLUSIVE: u16 = 0x4000;
pub const ROM0_THUNK_POOL_START: u16 = 0x3F00;
pub const ROMX_START: u16 = 0x4000;
pub const ROMX_END_EXCLUSIVE: u16 = 0x8000;

/// Physical or logical bank selected by layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BankIndex {
    Rom(u16),
    Sram(u8),
    Wram,
    Hram,
    Vram,
    Oam,
}

impl fmt::Display for BankIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rom(bank) => write!(f, "rom{bank}"),
            Self::Sram(bank) => write!(f, "sram{bank}"),
            Self::Wram => f.write_str("wram"),
            Self::Hram => f.write_str("hram"),
            Self::Vram => f.write_str("vram"),
            Self::Oam => f.write_str("oam"),
        }
    }
}

impl FromStr for BankIndex {
    type Err = BankIndexParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = value.strip_prefix("rom") {
            return Ok(Self::Rom(
                rest.parse()
                    .map_err(|_| BankIndexParseError(value.to_owned()))?,
            ));
        }
        if let Some(rest) = value.strip_prefix("sram") {
            return Ok(Self::Sram(
                rest.parse()
                    .map_err(|_| BankIndexParseError(value.to_owned()))?,
            ));
        }
        match value {
            "wram" => Ok(Self::Wram),
            "hram" => Ok(Self::Hram),
            "vram" => Ok(Self::Vram),
            "oam" => Ok(Self::Oam),
            _ => Err(BankIndexParseError(value.to_owned())),
        }
    }
}

impl Serialize for BankIndex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BankIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BankVisitor;

        impl Visitor<'_> for BankVisitor {
            type Value = BankIndex;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bank key such as rom0, rom3, sram1, wram, hram, vram, or oam")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(BankVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankIndexParseError(String);

impl fmt::Display for BankIndexParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid bank index {}", self.0)
    }
}

impl std::error::Error for BankIndexParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementProfile {
    StrictOneExpertPerBank,
    Budgeted { reserve_bytes_per_bank: u16 },
    PackedExperts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedPlacement {
    pub section_id: SectionId,
    pub bank: BankIndex,
    pub cpu_start: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedRange {
    pub bank: BankIndex,
    pub start: u16,
    pub end_inclusive: u16,
    pub reason: ReservedRangeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservedRangeReason {
    CartridgeHeader,
    ResetVector,
    InterruptVector,
    ThunkPool,
    UserPinned,
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
    /// item's stable order.
    pub alignment_padding: BTreeMap<ItemOrder, u16>,
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
                if self.cpu_end_exclusive() > ROM0_END_EXCLUSIVE as u32 {
                    return Err(LayoutError::SectionTooBig {
                        id: self.id,
                        size: u32::from(self.final_size),
                        bank_capacity: ROM0_END_EXCLUSIVE as u32,
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
                if self.cpu_end_exclusive() > ROMX_END_EXCLUSIVE as u32 {
                    return Err(LayoutError::SectionTooBig {
                        id: self.id,
                        size: u32::from(self.final_size),
                        bank_capacity: ROM_BANK_SIZE,
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
    pub reserved_ranges: Vec<ReservedRange>,
}

impl LayoutPlan {
    #[must_use]
    pub fn placement_for(&self, id: SectionId) -> Option<&PlacedSection> {
        self.sections.iter().find(|section| section.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    SectionTooBig {
        id: SectionId,
        size: u32,
        bank_capacity: u32,
    },
    NoBankFits {
        id: SectionId,
        role: SectionRole,
    },
    DuplicatePinned {
        section_id: SectionId,
    },
    PinnedPlacementOutOfRange {
        section_id: SectionId,
        cpu_start: u16,
        bank: BankIndex,
    },
    UserHeaderSectionRejected {
        section_id: SectionId,
    },
    PlacementCollision {
        section_id: SectionId,
        bank: BankIndex,
        start: u16,
        end_exclusive: u16,
    },
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
            Self::SectionTooBig {
                id,
                size,
                bank_capacity,
            } => write!(
                f,
                "section {} is {size} bytes and cannot fit in {bank_capacity} bytes",
                id.get()
            ),
            Self::NoBankFits { id, role } => {
                write!(f, "section {} with role {role:?} fits no bank", id.get())
            }
            Self::DuplicatePinned { section_id } => {
                write!(
                    f,
                    "section {} has duplicate pinned placements",
                    section_id.get()
                )
            }
            Self::PinnedPlacementOutOfRange {
                section_id,
                cpu_start,
                bank,
            } => write!(
                f,
                "section {} pinned at ${cpu_start:04X} is outside {bank}",
                section_id.get()
            ),
            Self::UserHeaderSectionRejected { section_id } => write!(
                f,
                "user section {} uses internal HeaderCartridge role",
                section_id.get()
            ),
            Self::PlacementCollision {
                section_id,
                bank,
                start,
                end_exclusive,
            } => write!(
                f,
                "section {} placement ${start:04X}..${end_exclusive:04X} collides in {bank}",
                section_id.get()
            ),
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

pub fn layout_into_banks(
    sections: &[LoweredSection],
    profile: PlacementProfile,
    pinned: &[PinnedPlacement],
) -> Result<LayoutPlan, LayoutError> {
    let pinned_by_section = pinned_map(pinned)?;
    let mut sections_out = Vec::with_capacity(sections.len());
    let mut free_bytes_per_bank = BTreeMap::new();
    let mut reserved_ranges = vec![
        ReservedRange {
            bank: BankIndex::Rom(0),
            start: 0x0000,
            end_inclusive: 0x00FF,
            reason: ReservedRangeReason::ResetVector,
        },
        ReservedRange {
            bank: BankIndex::Rom(0),
            start: 0x0100,
            end_inclusive: 0x014F,
            reason: ReservedRangeReason::CartridgeHeader,
        },
        ReservedRange {
            bank: BankIndex::Rom(0),
            start: ROM0_THUNK_POOL_START,
            end_inclusive: ROM0_END_EXCLUSIVE - 1,
            reason: ReservedRangeReason::ThunkPool,
        },
    ];
    for pin in pinned {
        reserved_ranges.push(ReservedRange {
            bank: pin.bank,
            start: pin.cpu_start,
            end_inclusive: pin.cpu_start,
            reason: ReservedRangeReason::UserPinned,
        });
    }
    let mut occupied = occupied_from_reserved(&reserved_ranges);
    let mut preplaced_pinned = BTreeMap::new();
    for pin in pinned {
        let Some(section) = sections.iter().find(|section| section.id == pin.section_id) else {
            continue;
        };
        if section.role == SectionRole::HeaderCartridge {
            return Err(LayoutError::UserHeaderSectionRejected {
                section_id: section.id,
            });
        }
        let space = space_for_bank(pin.bank, section.role);
        let placed = place_at(section, space, pin.bank, pin.cpu_start)?;
        validate_placed(&placed)?;
        if matches!(placed.bank, BankIndex::Rom(0))
            && placed.cpu_end_exclusive() > ROM0_THUNK_POOL_START as u32
        {
            return Err(LayoutError::SectionTooBig {
                id: placed.id,
                size: placed.final_size as u32,
                bank_capacity: u32::from(ROM0_THUNK_POOL_START),
            });
        }
        insert_occupied(&mut occupied, &placed)?;
        preplaced_pinned.insert(section.id, placed);
    }

    let mut rom0_cursor = 0x0150_u16;
    let mut bank_cursors: BTreeMap<u16, u16> = BTreeMap::new();
    let mut used_banks: BTreeSet<u16> = BTreeSet::from([0]);
    let mut next_strict_expert_bank = 1_u16;

    for section in sections {
        if section.role == SectionRole::HeaderCartridge {
            return Err(LayoutError::UserHeaderSectionRejected {
                section_id: section.id,
            });
        }

        let (space, bank, cursor) = if let Some(pin) = pinned_by_section.get(&section.id).copied() {
            let space = space_for_bank(pin.bank, section.role);
            (space, pin.bank, pin.cpu_start)
        } else {
            match section.role {
                SectionRole::Bank0Nucleus | SectionRole::Bank0Data => {
                    (AddressSpace::Rom0, BankIndex::Rom(0), rom0_cursor)
                }
                SectionRole::CommonBank | SectionRole::CommonData => {
                    let bank =
                        first_fit_bank(section, &mut bank_cursors, &occupied, profile, false)?;
                    (
                        AddressSpace::RomX,
                        BankIndex::Rom(bank),
                        *bank_cursors.entry(bank).or_insert(ROMX_START),
                    )
                }
                SectionRole::ExpertBank | SectionRole::ExpertData => {
                    let bank = match profile {
                        PlacementProfile::StrictOneExpertPerBank => {
                            let bank = next_strict_expert_bank.max(next_bank(&bank_cursors));
                            next_strict_expert_bank =
                                bank.checked_add(1).ok_or(LayoutError::NoBankFits {
                                    id: section.id,
                                    role: section.role,
                                })?;
                            bank_cursors.entry(bank).or_insert(ROMX_START);
                            bank
                        }
                        PlacementProfile::Budgeted { .. } | PlacementProfile::PackedExperts => {
                            first_fit_bank(section, &mut bank_cursors, &occupied, profile, true)?
                        }
                    };
                    (
                        AddressSpace::RomX,
                        BankIndex::Rom(bank),
                        *bank_cursors.entry(bank).or_insert(ROMX_START),
                    )
                }
                SectionRole::WramHotArena | SectionRole::WramOverlay => {
                    (AddressSpace::Wram, BankIndex::Wram, 0xC000)
                }
                SectionRole::HramFastFlags => (AddressSpace::Hram, BankIndex::Hram, 0xFF80),
                SectionRole::SramPersistent => (AddressSpace::Sram, BankIndex::Sram(0), 0xA000),
                SectionRole::VramOwnedByUi => (AddressSpace::Vram, BankIndex::Vram, 0x8000),
                SectionRole::OamOwnedByUi => (AddressSpace::Oam, BankIndex::Oam, 0xFE00),
                SectionRole::HeaderCartridge => unreachable!("rejected above"),
            }
        };

        let is_pinned = pinned_by_section.contains_key(&section.id);
        let placed = if is_pinned {
            preplaced_pinned
                .get(&section.id)
                .expect("pinned section was preplaced")
                .clone()
        } else {
            place_next(section, space, bank, cursor, &occupied, profile)?
        };
        validate_placed(&placed)?;
        if matches!(placed.bank, BankIndex::Rom(0))
            && placed.cpu_end_exclusive() > ROM0_THUNK_POOL_START as u32
        {
            return Err(LayoutError::SectionTooBig {
                id: placed.id,
                size: placed.final_size as u32,
                bank_capacity: u32::from(ROM0_THUNK_POOL_START),
            });
        }

        match placed.bank {
            BankIndex::Rom(0) => {
                rom0_cursor = rom0_cursor.max(placed.cpu_end_exclusive() as u16);
            }
            BankIndex::Rom(bank) => {
                used_banks.insert(bank);
                let cursor = bank_cursors.entry(bank).or_insert(ROMX_START);
                *cursor = (*cursor).max(placed.cpu_end_exclusive() as u16);
            }
            _ => {}
        }
        if !is_pinned {
            insert_occupied(&mut occupied, &placed)?;
        }
        sections_out.push(placed);
    }

    free_bytes_per_bank.insert(
        BankIndex::Rom(0),
        ROM0_THUNK_POOL_START as u32 - u32::from(rom0_cursor),
    );
    for bank in used_banks.iter().copied().filter(|bank| *bank != 0) {
        let cursor = *bank_cursors.get(&bank).unwrap_or(&ROMX_START);
        free_bytes_per_bank.insert(
            BankIndex::Rom(bank),
            ROMX_END_EXCLUSIVE as u32 - u32::from(cursor),
        );
    }

    let bank_count = used_banks.iter().max().copied().unwrap_or(1) + 1;
    Ok(LayoutPlan {
        sections: sections_out,
        bank_count: bank_count.next_power_of_two().max(2),
        free_bytes_per_bank,
        reserved_ranges,
    })
}

fn pinned_map(
    pinned: &[PinnedPlacement],
) -> Result<BTreeMap<SectionId, PinnedPlacement>, LayoutError> {
    let mut out = BTreeMap::new();
    for pin in pinned {
        if out.insert(pin.section_id, *pin).is_some() {
            return Err(LayoutError::DuplicatePinned {
                section_id: pin.section_id,
            });
        }
    }
    Ok(out)
}

fn first_fit_bank(
    section: &LoweredSection,
    bank_cursors: &mut BTreeMap<u16, u16>,
    occupied: &BTreeMap<BankIndex, Vec<(u32, u32)>>,
    profile: PlacementProfile,
    expert: bool,
) -> Result<u16, LayoutError> {
    let reserve = match profile {
        PlacementProfile::Budgeted {
            reserve_bytes_per_bank,
        } => u32::from(reserve_bytes_per_bank),
        PlacementProfile::StrictOneExpertPerBank | PlacementProfile::PackedExperts => 0,
    };
    let candidate_banks: Vec<u16> = if bank_cursors.is_empty() {
        vec![1]
    } else {
        bank_cursors
            .keys()
            .copied()
            .chain([next_bank(bank_cursors)])
            .collect()
    };

    for bank in candidate_banks {
        let cursor = *bank_cursors.entry(bank).or_insert(ROMX_START);
        let placed = place_next(
            section,
            AddressSpace::RomX,
            BankIndex::Rom(bank),
            cursor,
            occupied,
            profile,
        )?;
        let limit = ROMX_END_EXCLUSIVE as u32 - reserve;
        if placed.cpu_end_exclusive() <= limit {
            return Ok(bank);
        }
        if !expert && matches!(profile, PlacementProfile::StrictOneExpertPerBank) {
            continue;
        }
    }

    Err(LayoutError::NoBankFits {
        id: section.id,
        role: section.role,
    })
}

fn next_bank(bank_cursors: &BTreeMap<u16, u16>) -> u16 {
    bank_cursors.keys().next_back().copied().unwrap_or(0) + 1
}

fn space_for_bank(bank: BankIndex, role: SectionRole) -> AddressSpace {
    match bank {
        BankIndex::Rom(0) => AddressSpace::Rom0,
        BankIndex::Rom(_) => AddressSpace::RomX,
        BankIndex::Sram(_) => AddressSpace::Sram,
        BankIndex::Wram => AddressSpace::Wram,
        BankIndex::Hram => AddressSpace::Hram,
        BankIndex::Vram => AddressSpace::Vram,
        BankIndex::Oam => AddressSpace::Oam,
    }
    .or_role(role)
}

trait AddressSpaceRoleExt {
    fn or_role(self, _role: SectionRole) -> Self;
}

impl AddressSpaceRoleExt for AddressSpace {
    fn or_role(self, _role: SectionRole) -> Self {
        self
    }
}

fn place_at(
    section: &LoweredSection,
    space: AddressSpace,
    bank: BankIndex,
    cursor: u16,
) -> Result<PlacedSection, LayoutError> {
    let cpu_start = align_u16(cursor, section.align.get())?;
    let (final_size, alignment_padding) = section_size_from(section, cpu_start)?;
    if final_size > u16::MAX as u32 {
        return Err(LayoutError::SectionTooBig {
            id: section.id,
            size: final_size,
            bank_capacity: ROM_BANK_SIZE,
        });
    }
    Ok(PlacedSection {
        id: section.id,
        space,
        bank,
        cpu_start,
        final_size: final_size as u16,
        estimated_size: final_size as u16,
        alignment_padding,
    })
}

fn place_next(
    section: &LoweredSection,
    space: AddressSpace,
    bank: BankIndex,
    start: u16,
    occupied: &BTreeMap<BankIndex, Vec<(u32, u32)>>,
    profile: PlacementProfile,
) -> Result<PlacedSection, LayoutError> {
    let reserve = match profile {
        PlacementProfile::Budgeted {
            reserve_bytes_per_bank,
        } => u32::from(reserve_bytes_per_bank),
        PlacementProfile::StrictOneExpertPerBank | PlacementProfile::PackedExperts => 0,
    };
    let limit = match space {
        AddressSpace::Rom0 => u32::from(ROM0_THUNK_POOL_START),
        AddressSpace::RomX => ROMX_END_EXCLUSIVE as u32 - reserve,
        AddressSpace::Wram => 0xE000,
        AddressSpace::Hram => 0x1_0000,
        AddressSpace::Sram => 0xC000,
        AddressSpace::Vram => 0xA000,
        AddressSpace::Oam => 0xFEA0,
    };
    let mut cursor = start;
    loop {
        let placed = place_at(section, space, bank, cursor)?;
        if placed.cpu_end_exclusive() > limit {
            return Err(LayoutError::SectionTooBig {
                id: section.id,
                size: placed.final_size as u32,
                bank_capacity: limit,
            });
        }
        if let Some((_, end)) = first_overlap(&placed, occupied) {
            cursor = u16::try_from(end).map_err(|_| LayoutError::SectionTooBig {
                id: section.id,
                size: placed.final_size as u32,
                bank_capacity: limit,
            })?;
            continue;
        }
        return Ok(placed);
    }
}

fn section_size_from(
    section: &LoweredSection,
    cpu_start: u16,
) -> Result<(u32, BTreeMap<ItemOrder, u16>), LayoutError> {
    let mut cursor = u32::from(cpu_start);
    let mut padding_by_seq = BTreeMap::new();
    let mut items = Vec::new();
    items.extend(
        section
            .labels
            .iter()
            .map(|item| (item.order(), SizeItem::Label)),
    );
    items.extend(section.instrs.iter().map(|item| {
        (
            item.order(),
            SizeItem::Fixed(u32::from(item.data.byte_len())),
        )
    }));
    items.extend(section.data_blocks.iter().map(|item| {
        (
            item.order(),
            SizeItem::Fixed(match &item.data {
                DataBlock::Bytes(bytes) => bytes.len() as u32,
                DataBlock::Words(words) => words.len() as u32 * 2,
            }),
        )
    }));
    items.extend(
        section
            .alignments
            .iter()
            .map(|item| (item.order(), SizeItem::Align(item.data.0.get()))),
    );
    items.extend(section.branches.iter().map(|item| {
        (
            item.order(),
            SizeItem::Fixed(match item.data.kind {
                BranchKind::Jump => 3,
                BranchKind::Call => 3,
            }),
        )
    }));
    items.extend(section.legalization_ops.iter().map(|item| {
        (
            item.order(),
            SizeItem::Fixed(match item.data {
                LegalizationOp::FarCall { .. } => 3,
            }),
        )
    }));
    items.sort_by_key(|(order, _)| *order);

    for (order, item) in items {
        match item {
            SizeItem::Label => {}
            SizeItem::Fixed(size) => {
                cursor = cursor.checked_add(size).ok_or(LayoutError::SectionTooBig {
                    id: section.id,
                    size: u32::MAX,
                    bank_capacity: ROM_BANK_SIZE,
                })?
            }
            SizeItem::Align(align) => {
                let aligned = align_u32(cursor, align)?;
                let padding = aligned - cursor;
                padding_by_seq.insert(order, padding as u16);
                cursor = aligned;
            }
        }
    }
    Ok((cursor - u32::from(cpu_start), padding_by_seq))
}

fn occupied_from_reserved(
    reserved_ranges: &[ReservedRange],
) -> BTreeMap<BankIndex, Vec<(u32, u32)>> {
    let mut occupied: BTreeMap<BankIndex, Vec<(u32, u32)>> = BTreeMap::new();
    for range in reserved_ranges {
        if range.reason == ReservedRangeReason::UserPinned {
            continue;
        }
        occupied
            .entry(range.bank)
            .or_default()
            .push((u32::from(range.start), u32::from(range.end_inclusive) + 1));
    }
    for ranges in occupied.values_mut() {
        ranges.sort();
    }
    occupied
}

fn first_overlap(
    placed: &PlacedSection,
    occupied: &BTreeMap<BankIndex, Vec<(u32, u32)>>,
) -> Option<(u32, u32)> {
    let start = u32::from(placed.cpu_start);
    let end = placed.cpu_end_exclusive();
    occupied
        .get(&placed.bank)?
        .iter()
        .copied()
        .find(|(occupied_start, occupied_end)| start < *occupied_end && end > *occupied_start)
}

fn insert_occupied(
    occupied: &mut BTreeMap<BankIndex, Vec<(u32, u32)>>,
    placed: &PlacedSection,
) -> Result<(), LayoutError> {
    let start = u32::from(placed.cpu_start);
    let end = placed.cpu_end_exclusive();
    if first_overlap(placed, occupied).is_some() {
        return Err(LayoutError::PlacementCollision {
            section_id: placed.id,
            bank: placed.bank,
            start: placed.cpu_start,
            end_exclusive: u16::try_from(end).unwrap_or(u16::MAX),
        });
    }
    let ranges = occupied.entry(placed.bank).or_default();
    ranges.push((start, end));
    ranges.sort();
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum SizeItem {
    Label,
    Fixed(u32),
    Align(u16),
}

fn align_u16(value: u16, align: u16) -> Result<u16, LayoutError> {
    let aligned = align_u32(u32::from(value), align)?;
    u16::try_from(aligned).map_err(|_| LayoutError::PinnedPlacementOutOfRange {
        section_id: SectionId::new(u32::MAX),
        cpu_start: value,
        bank: BankIndex::Rom(0),
    })
}

fn align_u32(value: u32, align: u16) -> Result<u32, LayoutError> {
    let align = u32::from(align);
    Ok(value.div_ceil(align) * align)
}

fn validate_placed(placed: &PlacedSection) -> Result<(), LayoutError> {
    match placed.space {
        AddressSpace::Rom0 => {
            if placed.cpu_end_exclusive() > ROM0_END_EXCLUSIVE as u32 {
                return Err(LayoutError::SectionTooBig {
                    id: placed.id,
                    size: placed.final_size as u32,
                    bank_capacity: ROM0_END_EXCLUSIVE as u32,
                });
            }
        }
        AddressSpace::RomX => {
            if placed.cpu_start < ROMX_START
                || placed.cpu_end_exclusive() > ROMX_END_EXCLUSIVE as u32
            {
                return Err(LayoutError::SectionTooBig {
                    id: placed.id,
                    size: placed.final_size as u32,
                    bank_capacity: ROM_BANK_SIZE,
                });
            }
        }
        AddressSpace::Wram
        | AddressSpace::Hram
        | AddressSpace::Sram
        | AddressSpace::Vram
        | AddressSpace::Oam => {
            let (start, end) = match placed.space {
                AddressSpace::Wram => (0xC000_u16, 0xE000_u32),
                AddressSpace::Hram => (0xFF80_u16, 0x1_0000_u32),
                AddressSpace::Sram => (0xA000_u16, 0xC000_u32),
                AddressSpace::Vram => (0x8000_u16, 0xA000_u32),
                AddressSpace::Oam => (0xFE00_u16, 0xFEA0_u32),
                AddressSpace::Rom0 | AddressSpace::RomX => unreachable!("ROM handled above"),
            };
            if placed.cpu_start < start || placed.cpu_end_exclusive() > end {
                return Err(LayoutError::SectionTooBig {
                    id: placed.id,
                    size: placed.final_size as u32,
                    bank_capacity: end - u32::from(start),
                });
            }
        }
    }
    Ok(())
}

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

#[cfg(test)]
#[test]
fn rom_file_offset_rejects_bank_boundary_overflow() {
    let mut placed = PlacedSection {
        id: SectionId::new(1),
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start: 0x3FFE,
        final_size: 2,
        estimated_size: 2,
        alignment_padding: BTreeMap::new(),
    };
    assert_eq!(
        placed.rom_file_offset().expect("exact ROM0 end"),
        Some(0x3FFE)
    );
    placed.final_size = 3;
    assert!(matches!(
        placed.rom_file_offset(),
        Err(LayoutError::SectionTooBig { .. })
    ));

    placed.space = AddressSpace::RomX;
    placed.bank = BankIndex::Rom(1);
    placed.cpu_start = 0x7FFE;
    placed.final_size = 2;
    assert_eq!(
        placed.rom_file_offset().expect("exact ROMX end"),
        Some(0x4000 + 0x3FFE)
    );
    placed.final_size = 3;
    assert!(matches!(
        placed.rom_file_offset(),
        Err(LayoutError::SectionTooBig { .. })
    ));
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;
    use crate::isa::Instr;
    use crate::provenance::{InstrProvenance, PlanningStage};
    use crate::section::{LoweredSection, OrderedItem, SectionPrivilege};
    use crate::symbols::SymbolName;

    fn section(id: u32, role: SectionRole, instr_count: usize) -> LoweredSection {
        let prov = InstrProvenance::new(PlanningStage::Backend);
        LoweredSection {
            id: SectionId::new(id),
            role,
            name: SymbolName::section(role, SectionId::new(id)).expect("name"),
            privilege: SectionPrivilege::normal(),
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: instr_count as u32,
            labels: vec![],
            instrs: (0..instr_count)
                .map(|idx| OrderedItem::new(Instr::Nop, idx as u32, prov.clone()))
                .collect(),
            data_blocks: vec![],
            alignments: vec![],
            legalization_ops: vec![],
            branches: vec![],
        }
    }

    #[test]
    fn no_section_crosses_bank() {
        let sections = vec![
            section(1, SectionRole::Bank0Nucleus, 16),
            section(2, SectionRole::CommonBank, 32),
            section(3, SectionRole::ExpertBank, 64),
        ];
        let plan = layout_into_banks(&sections, PlacementProfile::PackedExperts, &[])
            .expect("layout succeeds");

        for placed in &plan.sections {
            match placed.space {
                AddressSpace::Rom0 => {
                    assert!(placed.cpu_end_exclusive() <= ROM0_END_EXCLUSIVE as u32)
                }
                AddressSpace::RomX => {
                    assert!(placed.cpu_start >= ROMX_START);
                    assert!(placed.cpu_end_exclusive() <= ROMX_END_EXCLUSIVE as u32);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn strict_one_expert_per_bank_semantics() {
        let sections = vec![
            section(1, SectionRole::ExpertBank, 4),
            section(2, SectionRole::ExpertBank, 4),
        ];
        let plan = layout_into_banks(&sections, PlacementProfile::StrictOneExpertPerBank, &[])
            .expect("layout succeeds");
        assert_ne!(plan.sections[0].bank, plan.sections[1].bank);
    }

    #[test]
    fn strict_expert_banks_skip_common_banks() {
        let sections = vec![
            section(1, SectionRole::CommonBank, 4),
            section(2, SectionRole::ExpertBank, 4),
        ];
        let plan = layout_into_banks(&sections, PlacementProfile::StrictOneExpertPerBank, &[])
            .expect("layout succeeds");
        assert_ne!(plan.sections[0].bank, plan.sections[1].bank);
    }

    #[test]
    fn budgeted_reserve_respected() {
        let sections = vec![section(1, SectionRole::CommonBank, 16)];
        let plan = layout_into_banks(
            &sections,
            PlacementProfile::Budgeted {
                reserve_bytes_per_bank: 256,
            },
            &[],
        )
        .expect("layout succeeds");
        assert!(plan.free_bytes_per_bank[&BankIndex::Rom(1)] >= 256);
    }

    #[test]
    fn layout_plan_json_round_trip_with_string_bank_keys() {
        let sections = vec![section(1, SectionRole::CommonBank, 16)];
        let plan = layout_into_banks(&sections, PlacementProfile::PackedExperts, &[])
            .expect("layout succeeds");
        let json = serde_json::to_string(&plan).expect("json serializes");
        assert!(json.contains("rom1"));
        let decoded: LayoutPlan = serde_json::from_str(&json).expect("json deserializes");
        assert_eq!(decoded, plan);
    }

    #[test]
    fn pinned_placements_cannot_overlap() {
        let sections = vec![
            section(1, SectionRole::Bank0Nucleus, 4),
            section(2, SectionRole::Bank0Nucleus, 4),
        ];
        let err = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[
                PinnedPlacement {
                    section_id: SectionId::new(1),
                    bank: BankIndex::Rom(0),
                    cpu_start: 0x0150,
                },
                PinnedPlacement {
                    section_id: SectionId::new(2),
                    bank: BankIndex::Rom(0),
                    cpu_start: 0x0152,
                },
            ],
        )
        .expect_err("overlapping pinned sections are rejected");

        assert!(matches!(err, LayoutError::PlacementCollision { .. }));
    }

    #[test]
    fn user_header_section_rejected() {
        let sections = vec![section(1, SectionRole::HeaderCartridge, 4)];
        let err = layout_into_banks(&sections, PlacementProfile::PackedExperts, &[])
            .expect_err("user header rejected");
        assert!(matches!(err, LayoutError::UserHeaderSectionRejected { .. }));
    }

    #[test]
    fn bank0_auto_placement_skips_pinned_sections() {
        let sections = vec![
            section(1, SectionRole::Bank0Nucleus, 4),
            section(2, SectionRole::Bank0Nucleus, 4),
        ];
        let plan = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[PinnedPlacement {
                section_id: SectionId::new(2),
                bank: BankIndex::Rom(0),
                cpu_start: 0x0150,
            }],
        )
        .expect("layout succeeds");

        let auto = plan
            .sections
            .iter()
            .find(|section| section.id == SectionId::new(1))
            .expect("auto section placed");
        assert!(auto.cpu_start >= 0x0154);
    }
}
