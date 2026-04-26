//! Typed assembly sections and section items.

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::effect::{
    MachineEffect, MachineEffectKind, PrivilegeClass, classify_effect, classify_pseudo_op,
    privilege_of,
};
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

/// Why a section privilege policy rejected an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrivilegeViolation {
    RequiredPrivilege {
        required: PrivilegeClass,
        section: PrivilegeClass,
    },
    InterruptDisabledNotAllowed,
    EffectKindNotAllowed {
        kind: MachineEffectKind,
    },
}

/// Existing section item rejected by a replacement privilege policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SectionPrivilegeError {
    pub item_index: usize,
    pub effect: MachineEffect,
    pub violation: PrivilegeViolation,
}

/// Section-level privilege and effect policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionPrivilege {
    pub default_privilege: PrivilegeClass,
    pub allows_interrupt_disabled: bool,
    pub allowed_effects: Option<BTreeSet<MachineEffectKind>>,
}

impl Default for SectionPrivilege {
    fn default() -> Self {
        Self::normal()
    }
}

impl SectionPrivilege {
    #[must_use]
    pub fn normal() -> Self {
        Self {
            default_privilege: PrivilegeClass::Normal,
            allows_interrupt_disabled: false,
            allowed_effects: None,
        }
    }

    #[must_use]
    pub fn privileged() -> Self {
        Self {
            default_privilege: PrivilegeClass::Privileged,
            allows_interrupt_disabled: true,
            allowed_effects: None,
        }
    }

    #[must_use]
    pub fn interrupt_handler() -> Self {
        Self {
            default_privilege: PrivilegeClass::InterruptHandler,
            allows_interrupt_disabled: false,
            allowed_effects: None,
        }
    }

    #[must_use]
    pub fn with_allows_interrupt_disabled(mut self, allows_interrupt_disabled: bool) -> Self {
        self.allows_interrupt_disabled = allows_interrupt_disabled;
        self
    }

    #[must_use]
    pub fn with_allowed_effects(
        mut self,
        allowed_effects: impl IntoIterator<Item = MachineEffectKind>,
    ) -> Self {
        self.allowed_effects = Some(allowed_effects.into_iter().collect());
        self
    }

    pub fn check_effect(&self, effect: MachineEffect) -> Result<(), PrivilegeViolation> {
        let kind = effect.kind();
        if let Some(allowed_effects) = &self.allowed_effects
            && !allowed_effects.contains(&kind)
        {
            return Err(PrivilegeViolation::EffectKindNotAllowed { kind });
        }

        if effect.disables_interrupts() && !self.allows_interrupt_disabled {
            return Err(PrivilegeViolation::InterruptDisabledNotAllowed);
        }

        let required = privilege_of(&effect);
        if self.allows_privilege(required) {
            Ok(())
        } else {
            Err(PrivilegeViolation::RequiredPrivilege {
                required,
                section: self.default_privilege,
            })
        }
    }

    #[must_use]
    pub fn permits_effect(&self, effect: MachineEffect) -> bool {
        self.check_effect(effect).is_ok()
    }

    #[must_use]
    pub const fn allows_privilege(&self, required: PrivilegeClass) -> bool {
        matches!(
            (self.default_privilege, required),
            (_, PrivilegeClass::Normal)
                | (PrivilegeClass::Privileged, PrivilegeClass::Privileged)
                | (
                    PrivilegeClass::InterruptHandler,
                    PrivilegeClass::InterruptHandler
                )
        )
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

/// Builder-local symbol id assigned when a label marker is emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SymbolId(u32);

impl SymbolId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<SymbolId> for u32 {
    fn from(value: SymbolId) -> Self {
        value.get()
    }
}

/// Bank class asserted or leased through runtime banking pseudo-ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbcBankClass {
    Rom,
    Sram,
}

/// Builder-local bank lease id used to thread pseudo-op ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LeaseId(u32);

impl LeaseId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Error returned when a bank lease marker cannot describe a legal MBC target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankLeaseSpecError {
    RomBankOutOfRange { bank: u16 },
    SramBankOutOfRange { bank: u16 },
}

impl fmt::Display for BankLeaseSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomBankOutOfRange { bank } => {
                write!(f, "ROM bank {bank} is outside MBC5 range 0..=511")
            }
            Self::SramBankOutOfRange { bank } => {
                write!(f, "SRAM bank {bank} is outside MBC5 range 0..=15")
            }
        }
    }
}

impl std::error::Error for BankLeaseSpecError {}

/// Bank lease marker accepted by the builder.
///
/// This is authoring intent, not the runtime BankLease ABI layout. The runtime
/// ABI/lowering bead owns concrete register writes and lease call sequences.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BankLeaseSpec {
    lease_id: LeaseId,
    class: MbcBankClass,
    bank: u16,
}

impl BankLeaseSpec {
    pub const MAX_ROM_BANK: u16 = 0x01FF;
    pub const MAX_SRAM_BANK: u16 = 0x000F;

    pub fn new(
        lease_id: LeaseId,
        class: MbcBankClass,
        bank: u16,
    ) -> Result<Self, BankLeaseSpecError> {
        match class {
            MbcBankClass::Rom if bank > Self::MAX_ROM_BANK => {
                return Err(BankLeaseSpecError::RomBankOutOfRange { bank });
            }
            MbcBankClass::Sram if bank > Self::MAX_SRAM_BANK => {
                return Err(BankLeaseSpecError::SramBankOutOfRange { bank });
            }
            _ => {}
        }

        Ok(Self {
            lease_id,
            class,
            bank,
        })
    }

    #[must_use]
    pub const fn lease_id(&self) -> LeaseId {
        self.lease_id
    }

    #[must_use]
    pub const fn class(&self) -> MbcBankClass {
        self.class
    }

    #[must_use]
    pub const fn bank(&self) -> u16 {
        self.bank
    }
}

/// Cooperative runtime yield marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YieldKind {
    PollInterrupts,
    FrameBoundary,
    Cooperative,
}

/// Trace probe identifier emitted by instrumentation pseudo-ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TraceProbeId(u32);

impl TraceProbeId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Trace probe verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeLevel {
    Trace,
    Debug,
    Info,
}

/// Pseudo-op markers consumed by the encoder/runtime lowering layer.
///
/// These markers do not write MBC registers directly. They preserve authoring
/// intent until the BankLease ABI lowering owns concrete instruction emission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PseudoOp {
    BankLease(BankLeaseSpec),
    BankRelease {
        lease_id: LeaseId,
    },
    FarCall {
        target: SymbolName,
        lease_chain: Vec<LeaseId>,
    },
    Yield {
        kind: YieldKind,
    },
    TraceProbe {
        id: TraceProbeId,
        level: ProbeLevel,
    },
    AssertBank {
        expected: MbcBankClass,
        expected_n: u8,
    },
}

/// Assembly section item. Every item carries provenance.
///
/// `SectionItem` is symbolic pre-layout IR. `Instr` items are concrete
/// instruction shapes, but labels, alignment directives, pseudo-ops, and raw
/// escape hatches still require later layout/relocation/encoding work.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SectionItem {
    Label {
        id: SymbolId,
        name: SymbolName,
        provenance: InstrProvenance,
    },
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
    Align {
        align: NonZeroU16,
        provenance: InstrProvenance,
    },
    Pseudo {
        op: PseudoOp,
        provenance: InstrProvenance,
    },
    Raw {
        bytes: Vec<u8>,
        provenance: InstrProvenance,
    },
}

impl SectionItem {
    #[must_use]
    pub fn label(id: SymbolId, name: SymbolName, provenance: InstrProvenance) -> Self {
        Self::Label {
            id,
            name,
            provenance,
        }
    }

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
    pub const fn align(align: NonZeroU16, provenance: InstrProvenance) -> Self {
        Self::Align { align, provenance }
    }

    #[must_use]
    pub fn pseudo(op: PseudoOp, provenance: InstrProvenance) -> Self {
        Self::Pseudo { op, provenance }
    }

    #[must_use]
    pub(crate) fn raw(bytes: Vec<u8>, provenance: InstrProvenance) -> Self {
        Self::Raw { bytes, provenance }
    }

    #[must_use]
    pub const fn provenance(&self) -> &InstrProvenance {
        match self {
            Self::Label { provenance, .. }
            | Self::Instr { provenance, .. }
            | Self::Db { provenance, .. }
            | Self::Dw { provenance, .. }
            | Self::Align { provenance, .. }
            | Self::Pseudo { provenance, .. }
            | Self::Raw { provenance, .. } => provenance,
        }
    }

    #[must_use]
    pub fn machine_effect(&self) -> Option<MachineEffect> {
        match self {
            Self::Instr { instr, .. } => Some(classify_effect(instr)),
            Self::Pseudo { op, .. } => Some(classify_pseudo_op(op)),
            Self::Raw { .. } => Some(MachineEffect::OpaqueBytes),
            Self::Label { .. } | Self::Db { .. } | Self::Dw { .. } | Self::Align { .. } => None,
        }
    }

    #[must_use]
    pub fn fixed_byte_len(&self) -> Option<u32> {
        match self {
            Self::Label { .. } => Some(0),
            Self::Align { .. } | Self::Pseudo { .. } => None,
            Self::Instr { instr, .. } => Some(u32::from(instr.byte_len())),
            Self::Db { bytes, .. } => Some(bytes.len() as u32),
            Self::Dw { words, .. } => Some((words.len() as u32) * 2),
            Self::Raw { bytes, .. } => Some(bytes.len() as u32),
        }
    }
}

/// Typed section with a validated alignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    id: SectionId,
    role: SectionRole,
    name: SymbolName,
    #[serde(default)]
    privilege: SectionPrivilege,
    items: Vec<SectionItem>,
    align: NonZeroU16,
    size_hint_bytes: Option<u32>,
}

impl Section {
    pub fn new(id: SectionId, role: SectionRole, name: SymbolName, align: NonZeroU16) -> Self {
        Self {
            id,
            role,
            name,
            privilege: SectionPrivilege::default(),
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

    pub fn with_privilege(mut self, privilege: SectionPrivilege) -> Self {
        self.try_set_privilege(privilege)
            .expect("section privilege rejected an existing item");
        self
    }

    pub fn try_with_privilege(
        mut self,
        privilege: SectionPrivilege,
    ) -> Result<Self, SectionPrivilegeError> {
        self.try_set_privilege(privilege)?;
        Ok(self)
    }

    pub(crate) fn set_privilege(
        &mut self,
        privilege: SectionPrivilege,
    ) -> Result<(), SectionPrivilegeError> {
        self.try_set_privilege(privilege)
    }

    fn try_set_privilege(
        &mut self,
        privilege: SectionPrivilege,
    ) -> Result<(), SectionPrivilegeError> {
        validate_items_for_privilege(&self.items, &privilege)?;
        self.privilege = privilege;
        Ok(())
    }

    pub(crate) fn push(&mut self, item: SectionItem) {
        self.items.push(item);
    }

    #[must_use]
    pub const fn id(&self) -> SectionId {
        self.id
    }

    #[must_use]
    pub const fn role(&self) -> SectionRole {
        self.role
    }

    #[must_use]
    pub fn name(&self) -> &SymbolName {
        &self.name
    }

    #[must_use]
    pub const fn privilege(&self) -> &SectionPrivilege {
        &self.privilege
    }

    #[must_use]
    pub fn items(&self) -> &[SectionItem] {
        &self.items
    }

    #[must_use]
    pub const fn align(&self) -> NonZeroU16 {
        self.align
    }

    #[must_use]
    pub const fn size_hint_bytes(&self) -> Option<u32> {
        self.size_hint_bytes
    }

    #[must_use]
    pub fn fixed_item_bytes(&self) -> Option<u32> {
        self.items
            .iter()
            .try_fold(0u32, |acc, item| item.fixed_byte_len().map(|len| acc + len))
    }
}

fn validate_items_for_privilege(
    items: &[SectionItem],
    privilege: &SectionPrivilege,
) -> Result<(), SectionPrivilegeError> {
    for (item_index, item) in items.iter().enumerate() {
        let Some(effect) = item.machine_effect() else {
            continue;
        };
        if let Err(violation) = privilege.check_effect(effect) {
            return Err(SectionPrivilegeError {
                item_index,
                effect,
                violation,
            });
        }
    }
    Ok(())
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
fn privilege_inheritance() {
    use crate::effect::{MachineEffect, MbcRegisterClass};

    let privileged_effect = MachineEffect::StoreToMbcRegister {
        reg: MbcRegisterClass::RomBankLow,
    };
    let normal_section = Section::new(
        SectionId::new(1),
        SectionRole::CommonBank,
        SymbolName::kernel("normal", 0).expect("section name"),
        NonZeroU16::new(1).expect("nonzero align"),
    );
    assert_eq!(
        normal_section.privilege(),
        &SectionPrivilege {
            default_privilege: PrivilegeClass::Normal,
            allows_interrupt_disabled: false,
            allowed_effects: None,
        }
    );
    assert_eq!(
        normal_section.privilege().check_effect(privileged_effect),
        Err(PrivilegeViolation::RequiredPrivilege {
            required: PrivilegeClass::Privileged,
            section: PrivilegeClass::Normal,
        })
    );

    let privileged_section = normal_section
        .clone()
        .with_privilege(SectionPrivilege::privileged());
    assert!(
        privileged_section
            .privilege()
            .permits_effect(privileged_effect)
    );

    let restricted_section = privileged_section.with_privilege(
        SectionPrivilege::privileged().with_allowed_effects([MachineEffectKind::PureCompute]),
    );
    assert_eq!(
        restricted_section
            .privilege()
            .check_effect(privileged_effect),
        Err(PrivilegeViolation::EffectKindNotAllowed {
            kind: MachineEffectKind::StoreToMbcRegister,
        })
    );

    let isr_section = SectionPrivilege::interrupt_handler();
    assert!(isr_section.permits_effect(MachineEffect::Reti));
    assert_eq!(
        SectionPrivilege::normal().check_effect(MachineEffect::Reti),
        Err(PrivilegeViolation::RequiredPrivilege {
            required: PrivilegeClass::InterruptHandler,
            section: PrivilegeClass::Normal,
        })
    );
    assert_eq!(
        isr_section.check_effect(MachineEffect::InterruptControl(
            crate::effect::InterruptControlOp::DisableInterrupts
        )),
        Err(PrivilegeViolation::InterruptDisabledNotAllowed)
    );
}

#[cfg(test)]
#[test]
fn section_items_carry_provenance_and_size() {
    use crate::effect::{MachineEffect, SystemCallKind};
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

    assert_eq!(section.fixed_item_bytes(), Some(7));
    assert_eq!(section.items()[0].provenance(), &provenance);
    section.push(SectionItem::pseudo(
        PseudoOp::Yield {
            kind: YieldKind::Cooperative,
        },
        provenance.clone(),
    ));
    assert_eq!(section.fixed_item_bytes(), None);
    assert_eq!(
        section.items()[3].machine_effect(),
        Some(MachineEffect::SystemCall(SystemCallKind::Yield))
    );
    let raw = SectionItem::raw(vec![0xF3], provenance);
    assert_eq!(raw.machine_effect(), Some(MachineEffect::OpaqueBytes));

    let encoded = serde_json::to_string(&section).expect("section serializes");
    let decoded: Section = serde_json::from_str(&encoded).expect("section deserializes");

    assert_eq!(decoded, section);
}
