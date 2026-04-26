//! Typed assembly sections and section items.

use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::isa::Instr;
use crate::provenance::InstrProvenance;
use crate::symbols::SymbolName;

/// Residency role used by placement, reachability, and listing passes.
///
/// This bead's role taxonomy supersedes the older sketch in `planv0.md` that
/// used names such as `RuntimeBank0` and `CommonKernel`. The newer names split
/// ROM/RAM/UI/header residency more directly for M0 layout and reachability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionRole {
    Bank0Nucleus,
    CommonBank,
    ExpertBank,
    WramHotArena,
    WramOverlay,
    HramFastFlags,
    SramPersistent,
    VramOwnedByUi,
    OamOwnedByUi,
    HeaderCartridge,
}

impl SectionRole {
    pub const ALL: [Self; 10] = [
        Self::Bank0Nucleus,
        Self::CommonBank,
        Self::ExpertBank,
        Self::WramHotArena,
        Self::WramOverlay,
        Self::HramFastFlags,
        Self::SramPersistent,
        Self::VramOwnedByUi,
        Self::OamOwnedByUi,
        Self::HeaderCartridge,
    ];

    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Bank0Nucleus => "bank0_nucleus",
            Self::CommonBank => "common_bank",
            Self::ExpertBank => "expert_bank",
            Self::WramHotArena => "wram_hot_arena",
            Self::WramOverlay => "wram_overlay",
            Self::HramFastFlags => "hram_fast_flags",
            Self::SramPersistent => "sram_persistent",
            Self::VramOwnedByUi => "vram_owned_by_ui",
            Self::OamOwnedByUi => "oam_owned_by_ui",
            Self::HeaderCartridge => "header_cartridge",
        }
    }
}

/// Monotone section identifier assigned by the host builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SectionId(u32);

impl SectionId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SectionId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<SectionId> for u32 {
    fn from(value: SectionId) -> Self {
        value.get()
    }
}

/// Assembly section item. Every item carries provenance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SectionItem {
    Instr {
        instr: Instr,
        provenance: InstrProvenance,
    },
    Db {
        bytes: Vec<u8>,
        provenance: InstrProvenance,
    },
    Dw {
        words: Vec<u16>,
        provenance: InstrProvenance,
    },
}

impl SectionItem {
    #[must_use]
    pub const fn instr(instr: Instr, provenance: InstrProvenance) -> Self {
        Self::Instr { instr, provenance }
    }

    #[must_use]
    pub fn db(bytes: impl Into<Vec<u8>>, provenance: InstrProvenance) -> Self {
        Self::Db {
            bytes: bytes.into(),
            provenance,
        }
    }

    #[must_use]
    pub fn dw(words: impl Into<Vec<u16>>, provenance: InstrProvenance) -> Self {
        Self::Dw {
            words: words.into(),
            provenance,
        }
    }

    #[must_use]
    pub const fn provenance(&self) -> &InstrProvenance {
        match self {
            Self::Instr { provenance, .. }
            | Self::Db { provenance, .. }
            | Self::Dw { provenance, .. } => provenance,
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> u32 {
        match self {
            Self::Instr { instr, .. } => u32::from(instr.byte_len()),
            Self::Db { bytes, .. } => bytes.len() as u32,
            Self::Dw { words, .. } => (words.len() as u32) * 2,
        }
    }
}

/// Typed section with a validated alignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: SectionId,
    pub role: SectionRole,
    pub name: SymbolName,
    pub items: Vec<SectionItem>,
    pub align: NonZeroU16,
    pub size_hint_bytes: Option<u32>,
}

impl Section {
    pub fn new(id: SectionId, role: SectionRole, name: SymbolName, align: NonZeroU16) -> Self {
        Self {
            id,
            role,
            name,
            items: Vec::new(),
            align,
            size_hint_bytes: None,
        }
    }

    #[must_use]
    pub fn with_size_hint_bytes(mut self, size_hint_bytes: u32) -> Self {
        self.size_hint_bytes = Some(size_hint_bytes);
        self
    }

    pub fn push(&mut self, item: SectionItem) {
        self.items.push(item);
    }

    #[must_use]
    pub fn item_bytes(&self) -> u32 {
        self.items.iter().map(SectionItem::byte_len).sum()
    }
}

#[cfg(test)]
#[test]
fn role_exhaustive() {
    assert_eq!(
        SectionRole::ALL,
        [
            SectionRole::Bank0Nucleus,
            SectionRole::CommonBank,
            SectionRole::ExpertBank,
            SectionRole::WramHotArena,
            SectionRole::WramOverlay,
            SectionRole::HramFastFlags,
            SectionRole::SramPersistent,
            SectionRole::VramOwnedByUi,
            SectionRole::OamOwnedByUi,
            SectionRole::HeaderCartridge,
        ]
    );
    assert_eq!(
        SectionRole::ALL.map(SectionRole::canonical_name),
        [
            "bank0_nucleus",
            "common_bank",
            "expert_bank",
            "wram_hot_arena",
            "wram_overlay",
            "hram_fast_flags",
            "sram_persistent",
            "vram_owned_by_ui",
            "oam_owned_by_ui",
            "header_cartridge",
        ]
    );

    let mut names = std::collections::BTreeSet::new();
    for role in SectionRole::ALL {
        assert!(
            names.insert(role.canonical_name()),
            "duplicate canonical role name: {}",
            role.canonical_name()
        );
    }
    assert_eq!(names.len(), SectionRole::ALL.len());
    assert_eq!(
        serde_json::to_string(&SectionRole::Bank0Nucleus).expect("role serializes"),
        r#""bank0_nucleus""#
    );
    assert_eq!(
        serde_json::from_str::<SectionRole>(r#""header_cartridge""#)
            .expect("role deserializes from canonical name"),
        SectionRole::HeaderCartridge
    );
}

#[cfg(test)]
#[test]
fn section_items_carry_provenance_and_size() {
    use crate::isa::Instr;
    use crate::provenance::{InstrProvenance, PlanningStage};

    let provenance = InstrProvenance::new(PlanningStage::Backend).with_source_op("emit_header");
    let mut section = Section::new(
        SectionId::new(9),
        SectionRole::HeaderCartridge,
        SymbolName::section(SectionRole::HeaderCartridge, SectionId::new(9)).expect("name"),
        NonZeroU16::new(16).expect("nonzero align"),
    )
    .with_size_hint_bytes(64);

    section.push(SectionItem::instr(Instr::Nop, provenance.clone()));
    section.push(SectionItem::db([0xCE, 0xED], provenance.clone()));
    section.push(SectionItem::dw([0x1234, 0xCAFE], provenance.clone()));

    assert_eq!(section.item_bytes(), 7);
    assert_eq!(section.items[0].provenance(), &provenance);

    let encoded = serde_json::to_string(&section).expect("section serializes");
    let decoded: Section = serde_json::from_str(&encoded).expect("section deserializes");

    assert_eq!(decoded, section);
}
