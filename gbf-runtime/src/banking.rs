//! BankLease / BankGuard ABI and MBC5 banking lowerers.
//!
//! This module is the only runtime authoring path that emits MBC5 register
//! writes. Public callers acquire and release typed leases; the raw register
//! writing helpers stay crate-private and are reached by the banking lowerer.

extern crate alloc;

use alloc::collections::BTreeMap;
use core::cell::RefCell;
use core::fmt;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub use gbf_abi::interrupt::InterruptPolicy;
use gbf_asm::builder::{Builder, BuilderError};
use gbf_asm::effect::{MachineEffect, PrivilegeClass, classify_effect};
use gbf_asm::isa::{AluSrc8, Cond, DirectAddr, HighDirectOffset, Instr, Reg8, RstVector};
use gbf_asm::lowering::{
    DispositionPreLayoutOpLowering, FragmentItem, LoweredFragment, LoweringContext,
    LoweringDisposition, LoweringError, PreLayoutOpLowering,
};
use gbf_asm::provenance::InstrProvenance;
pub use gbf_asm::section::{BankLeaseSpec, LeaseGeneration, LeaseId, LeaseLifetime};
use gbf_asm::section::{
    BankReleaseDisposition, MbcBankClass, PreLayoutOp, Section, SectionId, SectionPrivilege,
    SectionRole,
};
use gbf_asm::symbols::SymbolName;
use gbf_hw::{mbc5, memory};
use serde::{Deserialize, Serialize};

pub const HRAM_SHADOW_BASE: u16 = memory::HRAM_BASE;
pub const HRAM_ADDR_CURRENT_ROM_BANK_LO: u16 = HRAM_SHADOW_BASE;
pub const HRAM_ADDR_CURRENT_ROM_BANK_HI: u16 = HRAM_SHADOW_BASE + 1;
pub const HRAM_ADDR_CURRENT_SRAM_BANK: u16 = HRAM_SHADOW_BASE + 2;
pub const HRAM_ADDR_SRAM_ENABLED: u16 = HRAM_SHADOW_BASE + 3;
pub const HRAM_BANKING_SHADOW_END_EXCLUSIVE: u16 = HRAM_SHADOW_BASE + 4;

pub const HRAM_LDH_CURRENT_ROM_BANK_LO: u8 = 0x80;
pub const HRAM_LDH_CURRENT_ROM_BANK_HI: u8 = 0x81;
pub const HRAM_LDH_CURRENT_SRAM_BANK: u8 = 0x82;
pub const HRAM_LDH_SRAM_ENABLED: u8 = 0x83;

const BANKING_SOURCE_OP: &str = "gbf-runtime::banking";
const BANKING_PROVENANCE_NOTE_PREFIX: &str = "f-a4-banking-lowered";
static NEXT_LEASE_GENERATION: AtomicU32 = AtomicU32::new(1);
static NEXT_LOWERING_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionResidency {
    FixedRom0,
    Hram,
    SwitchableRom,
    Wram,
    Other,
}

impl SectionResidency {
    #[must_use]
    pub const fn from_section_role(role: SectionRole) -> Self {
        match role {
            SectionRole::Bank0Nucleus | SectionRole::Bank0Data | SectionRole::HeaderCartridge => {
                Self::FixedRom0
            }
            SectionRole::HramFastFlags => Self::Hram,
            SectionRole::CommonBank | SectionRole::ExpertBank => Self::SwitchableRom,
            SectionRole::WramHotArena | SectionRole::WramOverlay => Self::Wram,
            SectionRole::CommonData
            | SectionRole::ExpertData
            | SectionRole::SramPersistent
            | SectionRole::VramOwnedByUi
            | SectionRole::OamOwnedByUi => Self::Other,
        }
    }

    #[must_use]
    pub const fn allows_banking_emit(self) -> bool {
        matches!(self, Self::FixedRom0 | Self::Hram)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidatedBankLeaseSpec {
    class: MbcBankClass,
    bank: u16,
    lifetime: LeaseLifetime,
}

impl ValidatedBankLeaseSpec {
    pub fn for_rom_switchable(
        bank_n: u16,
        lifetime: LeaseLifetime,
    ) -> Result<Self, BankAbiViolation> {
        if bank_n == 0 {
            return Err(BankAbiViolation::RomBankZeroReservedByAbi);
        }
        if bank_n > BankLeaseSpec::MAX_ROM_BANK {
            return Err(BankAbiViolation::RomBankOutOfRange { bank: bank_n });
        }
        Ok(Self {
            class: MbcBankClass::Rom,
            bank: bank_n,
            lifetime,
        })
    }

    pub fn for_sram(bank_n: u8, lifetime: LeaseLifetime) -> Result<Self, BankAbiViolation> {
        Self::for_sram_bank(u16::from(bank_n), lifetime)
    }

    pub fn for_sram_bank(bank_n: u16, lifetime: LeaseLifetime) -> Result<Self, BankAbiViolation> {
        if bank_n > BankLeaseSpec::MAX_SRAM_BANK {
            return Err(BankAbiViolation::SramBankOutOfRange { bank: bank_n });
        }
        Ok(Self {
            class: MbcBankClass::Sram,
            bank: bank_n,
            lifetime,
        })
    }

    #[must_use]
    pub const fn class(&self) -> MbcBankClass {
        self.class
    }

    #[must_use]
    pub const fn bank(&self) -> u16 {
        self.bank
    }

    #[must_use]
    pub const fn lifetime(&self) -> LeaseLifetime {
        self.lifetime
    }

    fn to_pre_layout_spec(
        &self,
        lease_id: LeaseId,
        generation: LeaseGeneration,
    ) -> Result<BankLeaseSpec, BankAbiViolation> {
        BankLeaseSpec::new(lease_id, generation, self.class, self.bank, self.lifetime).map_err(
            |err| match err {
                gbf_asm::section::BankLeaseSpecError::RomBankOutOfRange { bank } => {
                    BankAbiViolation::RomBankOutOfRange { bank }
                }
                gbf_asm::section::BankLeaseSpecError::SramBankOutOfRange { bank } => {
                    BankAbiViolation::SramBankOutOfRange { bank }
                }
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum ReturnState {
    Rom(ReturnRomBank),
    Sram(ReturnSramState),
    KeepCurrent(KeepCurrentProof),
}

impl ReturnState {
    fn disposition(self) -> Result<BankReleaseDisposition, BankAbiViolation> {
        match self {
            Self::Rom(ReturnRomBank::Bank1) => Ok(BankReleaseDisposition::RomBank1),
            Self::Rom(ReturnRomBank::Manual(bank)) => {
                validate_return_rom_bank(bank)?;
                Ok(BankReleaseDisposition::RomManual(bank))
            }
            Self::Sram(ReturnSramState::Disable) => Ok(BankReleaseDisposition::SramDisable),
            Self::Sram(ReturnSramState::Bank(bank)) => {
                validate_sram_bank(u16::from(bank))?;
                Ok(BankReleaseDisposition::SramBank(bank))
            }
            Self::KeepCurrent(_) => Ok(BankReleaseDisposition::KeepCurrent),
        }
    }

    const fn return_class_name(self) -> &'static str {
        match self {
            Self::Rom(_) => "rom",
            Self::Sram(_) => "sram",
            Self::KeepCurrent(_) => "keep_current",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnRomBank {
    Bank1,
    Manual(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnSramState {
    Disable,
    Bank(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct KeepCurrentProof(pub(crate) ());

#[must_use = "a BankGuard must be explicitly released with release_bank or try_finish will fail"]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct BankGuard {
    lease_id: LeaseId,
    source_section_id: SectionId,
    generation: LeaseGeneration,
    class: MbcBankClass,
    bank: u16,
    lifetime: LeaseLifetime,
}

impl BankGuard {
    #[must_use]
    pub const fn lease_id(&self) -> LeaseId {
        self.lease_id
    }

    #[must_use]
    pub const fn generation(&self) -> LeaseGeneration {
        self.generation
    }

    #[must_use]
    pub const fn source_section_id(&self) -> SectionId {
        self.source_section_id
    }

    #[must_use]
    pub const fn class(&self) -> MbcBankClass {
        self.class
    }

    #[must_use]
    pub const fn bank(&self) -> u16 {
        self.bank
    }

    #[must_use]
    pub const fn lifetime(&self) -> LeaseLifetime {
        self.lifetime
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankLease {
    pub id: LeaseId,
    pub spec: BankLeaseSpec,
    pub lifetime: LeaseLifetime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankAbiViolation {
    RomBankOutOfRange {
        bank: u16,
    },
    SramBankOutOfRange {
        bank: u16,
    },
    RomBankZeroReservedByAbi,
    LeaseSpecWrongClass {
        expected: MbcBankClass,
        found: MbcBankClass,
    },
    ManualLeaseInYieldingSection {
        section: SectionId,
    },
    IsrCannotAcquire {
        section: SectionId,
    },
    SectionNotPrivileged {
        section: SectionId,
        found: PrivilegeClass,
    },
    BankingPrimitiveNotFixedResident {
        section: SectionId,
        residency: SectionResidency,
    },
    UnreleasedLease {
        lease: LeaseId,
    },
    SramBankAcquiredWhileDisabled,
    ReturnBankOutOfRange {
        bank: u16,
    },
    ReturnStateWrongClass {
        lease_class: MbcBankClass,
        return_class: String,
    },
    NestedLeaseNotSupported {
        class: MbcBankClass,
        existing: LeaseId,
        attempted: LeaseId,
    },
    UnsupportedRestoringLifetime {
        lifetime: LeaseLifetime,
    },
    RumbleCartProfileRejected,
    StaleBankGuard {
        lease: LeaseId,
    },
    KeepCurrentProofMissing {
        lease: LeaseId,
    },
    MbcWriteOutsideBankingProvenance {
        section: SectionId,
    },
}

impl fmt::Display for BankAbiViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomBankOutOfRange { bank } => {
                write!(f, "ROM bank {bank} is outside MBC5 range 0..=511")
            }
            Self::SramBankOutOfRange { bank } => {
                write!(f, "SRAM bank {bank} is outside MBC5 range 0..=15")
            }
            Self::RomBankZeroReservedByAbi => {
                f.write_str("ROM bank 0 is reserved by the BankLease ABI")
            }
            Self::LeaseSpecWrongClass { expected, found } => {
                write!(f, "lease spec class {found:?} does not match {expected:?}")
            }
            Self::ManualLeaseInYieldingSection { section } => {
                write!(f, "manual lease in yielding section {}", section.get())
            }
            Self::IsrCannotAcquire { section } => {
                write!(
                    f,
                    "ISR section {} may not acquire bank leases",
                    section.get()
                )
            }
            Self::SectionNotPrivileged { section, found } => {
                write!(f, "section {} is {found:?}, not Privileged", section.get())
            }
            Self::BankingPrimitiveNotFixedResident { section, residency } => write!(
                f,
                "section {} has residency {residency:?}; banking emits require fixed ROM0 or HRAM",
                section.get()
            ),
            Self::UnreleasedLease { lease } => write!(f, "lease {} was not released", lease.get()),
            Self::SramBankAcquiredWhileDisabled => {
                f.write_str("SRAM bank acquired before SRAM was enabled")
            }
            Self::ReturnBankOutOfRange { bank } => {
                write!(f, "return ROM bank {bank} is outside ABI range 1..=511")
            }
            Self::ReturnStateWrongClass {
                lease_class,
                return_class,
            } => write!(
                f,
                "return state class {return_class} does not match lease class {lease_class:?}"
            ),
            Self::NestedLeaseNotSupported {
                class,
                existing,
                attempted,
            } => write!(
                f,
                "nested {class:?} lease is unsupported: existing {}, attempted {}",
                existing.get(),
                attempted.get()
            ),
            Self::UnsupportedRestoringLifetime { lifetime } => write!(
                f,
                "lease lifetime {lifetime:?} requires scheduler restoration not owned by F-A4"
            ),
            Self::RumbleCartProfileRejected => f.write_str("MBC5 rumble cartridges are rejected"),
            Self::StaleBankGuard { lease } => {
                write!(f, "bank guard for lease {} is stale", lease.get())
            }
            Self::KeepCurrentProofMissing { lease } => write!(
                f,
                "keep-current release for lease {} lacks trusted runtime proof",
                lease.get()
            ),
            Self::MbcWriteOutsideBankingProvenance { section } => write!(
                f,
                "MBC write in section {} lacks gbf-runtime::banking provenance",
                section.get()
            ),
        }
    }
}

impl core::error::Error for BankAbiViolation {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankingEmitError {
    Abi(BankAbiViolation),
    Builder(String),
    ShadowOffsetOutOfRange { offset: u8 },
    EnabledPolicyForRomAcquire,
    EnabledPolicyForSramAcquire,
    EnabledPolicyForSramEnable,
    EnabledPolicyForSramDisable,
    SymbolName(String),
}

impl fmt::Display for BankingEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Abi(err) => write!(f, "{err}"),
            Self::Builder(err) => write!(f, "builder rejected banking emit: {err}"),
            Self::ShadowOffsetOutOfRange { offset } => {
                write!(f, "HRAM shadow offset ${offset:02X} is not banking-owned")
            }
            Self::EnabledPolicyForRomAcquire => {
                f.write_str("ROM acquire requires Disabled or ShortCriticalSection policy")
            }
            Self::EnabledPolicyForSramAcquire => {
                f.write_str("SRAM acquire requires Disabled or ShortCriticalSection policy")
            }
            Self::EnabledPolicyForSramEnable => {
                f.write_str("SRAM enable requires Disabled or ShortCriticalSection policy")
            }
            Self::EnabledPolicyForSramDisable => {
                f.write_str("SRAM disable requires Disabled or ShortCriticalSection policy")
            }
            Self::SymbolName(err) => write!(f, "invalid banking symbol: {err}"),
        }
    }
}

impl core::error::Error for BankingEmitError {}

impl From<BankAbiViolation> for BankingEmitError {
    fn from(value: BankAbiViolation) -> Self {
        Self::Abi(value)
    }
}

impl From<BuilderError> for BankingEmitError {
    fn from(value: BuilderError) -> Self {
        Self::Builder(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowRegisters {
    pub current_rom_bank: u16,
    pub current_sram_bank: u8,
    pub sram_enabled: bool,
}

pub(crate) fn emit_store_bank_shadow_byte_from_a(
    b: &mut Builder,
    offset: HighDirectOffset,
) -> Result<(), BankingEmitError> {
    validate_banking_shadow_offset(offset)?;
    b.try_emit(Instr::LdHighDirectFromA { offset })?;
    Ok(())
}

pub(crate) fn emit_store_bank_shadow_byte_imm(
    b: &mut Builder,
    offset: HighDirectOffset,
    value: u8,
) -> Result<(), BankingEmitError> {
    validate_banking_shadow_offset(offset)?;
    b.try_emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: value,
    })?;
    emit_store_bank_shadow_byte_from_a(b, offset)
}

pub fn emit_load_bank_shadow_byte_into_a(
    b: &mut Builder,
    offset: HighDirectOffset,
) -> Result<(), BankingEmitError> {
    validate_banking_shadow_offset(offset)?;
    b.try_emit(Instr::LdAFromHighDirect { offset })?;
    Ok(())
}

pub fn lower_banking_shadow_zero_init(b: &mut Builder) -> Result<(), BankingEmitError> {
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        emit_store_bank_shadow_byte_imm(b, HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_LO), 0)?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_HI))?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_CURRENT_SRAM_BANK))?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_SRAM_ENABLED))?;
        Ok::<(), BankingEmitError>(())
    })
}

pub fn lease_rom_switchable(
    b: &mut Builder,
    spec: ValidatedBankLeaseSpec,
) -> Result<BankGuard, BankingEmitError> {
    if spec.class() != MbcBankClass::Rom {
        return Err(BankAbiViolation::LeaseSpecWrongClass {
            expected: MbcBankClass::Rom,
            found: spec.class(),
        }
        .into());
    }
    lease_bank(b, spec)
}

pub fn lease_sram(
    b: &mut Builder,
    spec: ValidatedBankLeaseSpec,
) -> Result<BankGuard, BankingEmitError> {
    if spec.class() != MbcBankClass::Sram {
        return Err(BankAbiViolation::LeaseSpecWrongClass {
            expected: MbcBankClass::Sram,
            found: spec.class(),
        }
        .into());
    }
    lease_bank(b, spec)
}

pub fn release_bank(
    b: &mut Builder,
    guard: BankGuard,
    return_state: ReturnState,
) -> Result<(), BankingEmitError> {
    validate_return_state_class(guard.class, return_state)?;
    validate_guard_matches_builder(b, &guard)?;
    let return_to = return_state.disposition()?;
    let provenance = untrusted_banking_provenance(b.current_provenance());
    b.with_provenance(provenance, |b| {
        b.try_bank_release_to(guard.lease_id, return_to)
    })
    .map_err(|err| match err {
        BuilderError::UnknownLease { lease_id } => {
            BankingEmitError::Abi(BankAbiViolation::StaleBankGuard { lease: lease_id })
        }
        other => BankingEmitError::from(other),
    })
}

fn lease_bank(
    b: &mut Builder,
    spec: ValidatedBankLeaseSpec,
) -> Result<BankGuard, BankingEmitError> {
    let lease_id = b.allocate_lease_id();
    let generation = next_lease_generation();
    if let Some(existing) = b.first_active_lease_id() {
        return Err(BankAbiViolation::NestedLeaseNotSupported {
            class: spec.class(),
            existing,
            attempted: lease_id,
        }
        .into());
    }

    let pre_layout = spec.to_pre_layout_spec(lease_id, generation)?;
    let provenance = untrusted_banking_provenance(b.current_provenance());
    b.with_provenance(provenance, |b| b.try_bank_lease(pre_layout))?;
    Ok(BankGuard {
        lease_id,
        source_section_id: b.section_id(),
        generation,
        class: spec.class(),
        bank: spec.bank(),
        lifetime: spec.lifetime(),
    })
}

fn next_lease_generation() -> LeaseGeneration {
    LeaseGeneration(NEXT_LEASE_GENERATION.fetch_add(1, Ordering::Relaxed))
}

fn validate_guard_matches_builder(b: &Builder, guard: &BankGuard) -> Result<(), BankingEmitError> {
    let active = b
        .active_lease_spec(guard.lease_id)
        .ok_or(BankingEmitError::Abi(BankAbiViolation::StaleBankGuard {
            lease: guard.lease_id,
        }))?;
    let matches_active = b.section_id() == guard.source_section_id
        && active.generation() == guard.generation
        && active.class() == guard.class
        && active.bank() == guard.bank
        && active.lifetime() == guard.lifetime;
    if matches_active {
        Ok(())
    } else {
        Err(BankingEmitError::Abi(BankAbiViolation::StaleBankGuard {
            lease: guard.lease_id,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptSafety {
    pub kind: InterruptSafetyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptSafetyKind {
    InterruptDisabled,
    InterruptEnabledBank0Only,
    InterruptHandler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptSafetyError {
    ConflictingDeclaration {
        section: SectionId,
        old: InterruptSafetyKind,
        new: InterruptSafetyKind,
    },
}

impl fmt::Display for InterruptSafetyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingDeclaration { section, old, new } => write!(
                f,
                "conflicting safety declaration for section {}: was {old:?}, now {new:?}",
                section.get()
            ),
        }
    }
}

impl core::error::Error for InterruptSafetyError {}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptSafetyTable {
    pub by_section: BTreeMap<SectionId, InterruptSafety>,
}

impl InterruptSafetyTable {
    pub fn declare(
        &mut self,
        section: &Section,
        kind: InterruptSafetyKind,
    ) -> Result<(), InterruptSafetyError> {
        let safety = InterruptSafety { kind };
        match self.by_section.get(&section.id()).copied() {
            Some(old) if old.kind != kind => Err(InterruptSafetyError::ConflictingDeclaration {
                section: section.id(),
                old: old.kind,
                new: kind,
            }),
            Some(_) => Ok(()),
            None => {
                self.by_section.insert(section.id(), safety);
                Ok(())
            }
        }
    }

    #[must_use]
    pub fn lookup(&self, section: SectionId) -> Option<InterruptSafety> {
        self.by_section.get(&section).copied()
    }

    pub fn export(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("InterruptSafetyTable serializes")
    }
}

pub fn mark_isr_unreachable(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError> {
    table.declare(section, InterruptSafetyKind::InterruptDisabled)
}

pub fn mark_isr_reachable(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError> {
    table.declare(section, InterruptSafetyKind::InterruptEnabledBank0Only)
}

pub fn mark_isr(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError> {
    table.declare(section, InterruptSafetyKind::InterruptHandler)
}

pub fn check_lease_emission_legal(
    section: &Section,
    safety: InterruptSafety,
    residency: SectionResidency,
) -> Result<(), BankAbiViolation> {
    check_lease_emission_legal_parts(section.id(), section.privilege(), safety, residency)
}

fn check_lease_emission_legal_parts(
    section: SectionId,
    privilege: &SectionPrivilege,
    safety: InterruptSafety,
    residency: SectionResidency,
) -> Result<(), BankAbiViolation> {
    if safety.kind == InterruptSafetyKind::InterruptHandler {
        return Err(BankAbiViolation::IsrCannotAcquire { section });
    }
    if privilege.default_privilege != PrivilegeClass::Privileged {
        return Err(BankAbiViolation::SectionNotPrivileged {
            section,
            found: privilege.default_privilege,
        });
    }
    if !residency.allows_banking_emit() {
        return Err(BankAbiViolation::BankingPrimitiveNotFixedResident { section, residency });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveLease {
    lease: BankLease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BankingLoweringState {
    pub(crate) current_rom_bank: Option<u16>,
    pub(crate) current_sram_bank: Option<u8>,
    pub(crate) sram_enabled: bool,
    pub(crate) active_leases: BTreeMap<LeaseId, ActiveLease>,
}

impl Default for BankingLoweringState {
    fn default() -> Self {
        Self {
            current_rom_bank: Some(1),
            current_sram_bank: None,
            sram_enabled: false,
            active_leases: BTreeMap::new(),
        }
    }
}

impl BankingLoweringState {
    fn active_lease_id(&self) -> Option<LeaseId> {
        self.active_leases.keys().next().copied()
    }
}

pub(crate) fn lower_enable_sram(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError> {
    guard_policy(policy, BankingPolicyOp::SramEnable)?;
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        emit_policy_prologue(b, policy)?;
        b.try_emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: mbc5::MBC5_RAM_ENABLE_VALUE,
        })?;
        b.try_emit(Instr::LdDirectFromA {
            addr: direct(mbc5::MBC5_RAMG_BASE),
        })?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_SRAM_ENABLED))?;
        emit_policy_epilogue(b, policy)?;
        Ok::<(), BankingEmitError>(())
    })?;
    state.sram_enabled = true;
    Ok(())
}

pub(crate) fn lower_disable_sram(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError> {
    guard_policy(policy, BankingPolicyOp::SramDisable)?;
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        emit_policy_prologue(b, policy)?;
        b.try_emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: mbc5::MBC5_RAM_DISABLE_VALUE,
        })?;
        b.try_emit(Instr::LdDirectFromA {
            addr: direct(mbc5::MBC5_RAMG_BASE),
        })?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_SRAM_ENABLED))?;
        emit_policy_epilogue(b, policy)?;
        Ok::<(), BankingEmitError>(())
    })?;
    state.sram_enabled = false;
    state.current_sram_bank = None;
    Ok(())
}

pub(crate) fn lower_acquire_rom_bank(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    bank: u16,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError> {
    if bank == 0 {
        return Err(BankAbiViolation::RomBankZeroReservedByAbi.into());
    }
    validate_return_rom_bank(bank)?;
    guard_policy(policy, BankingPolicyOp::RomAcquire)?;
    let lo = bank as u8;
    let hi = ((bank >> 8) & 0x01) as u8;
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        emit_policy_prologue(b, policy)?;
        b.try_emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: lo,
        })?;
        b.try_emit(Instr::LdDirectFromA {
            addr: direct(mbc5::MBC5_BANK1_BASE),
        })?;
        b.try_emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: hi,
        })?;
        b.try_emit(Instr::LdDirectFromA {
            addr: direct(mbc5::MBC5_BANK2_BASE),
        })?;
        emit_store_bank_shadow_byte_imm(
            b,
            HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_LO),
            lo,
        )?;
        emit_store_bank_shadow_byte_imm(
            b,
            HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_HI),
            hi,
        )?;
        emit_policy_epilogue(b, policy)?;
        Ok::<(), BankingEmitError>(())
    })?;
    state.current_rom_bank = Some(bank);
    Ok(())
}

pub(crate) fn lower_acquire_sram_bank(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    bank: u8,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError> {
    validate_sram_bank(u16::from(bank))?;
    if !state.sram_enabled {
        return Err(BankAbiViolation::SramBankAcquiredWhileDisabled.into());
    }
    guard_policy(policy, BankingPolicyOp::SramAcquire)?;
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        emit_policy_prologue(b, policy)?;
        b.try_emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: bank,
        })?;
        b.try_emit(Instr::LdDirectFromA {
            addr: direct(mbc5::MBC5_RAMB_BASE),
        })?;
        emit_store_bank_shadow_byte_from_a(b, HighDirectOffset::new(HRAM_LDH_CURRENT_SRAM_BANK))?;
        emit_policy_epilogue(b, policy)?;
        Ok::<(), BankingEmitError>(())
    })?;
    state.current_sram_bank = Some(bank);
    Ok(())
}

pub(crate) fn lower_release(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    lease: &BankLease,
    return_to: BankReleaseDisposition,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError> {
    match (lease.spec.class(), return_to) {
        (_, BankReleaseDisposition::KeepCurrent) => emit_release_label(b, lease.id),
        (MbcBankClass::Rom, BankReleaseDisposition::RomBank1) => {
            lower_acquire_rom_bank(b, state, 1, policy)
        }
        (MbcBankClass::Rom, BankReleaseDisposition::RomManual(bank)) => {
            lower_acquire_rom_bank(b, state, bank, policy)
        }
        (MbcBankClass::Sram, BankReleaseDisposition::SramDisable) => {
            lower_disable_sram(b, state, policy)
        }
        (MbcBankClass::Sram, BankReleaseDisposition::SramBank(bank)) => {
            lower_acquire_sram_bank(b, state, bank, policy)
        }
        (lease_class, BankReleaseDisposition::RomBank1 | BankReleaseDisposition::RomManual(_)) => {
            Err(BankAbiViolation::ReturnStateWrongClass {
                lease_class,
                return_class: "rom".to_owned(),
            }
            .into())
        }
        (
            lease_class,
            BankReleaseDisposition::SramDisable | BankReleaseDisposition::SramBank(_),
        ) => Err(BankAbiViolation::ReturnStateWrongClass {
            lease_class,
            return_class: "sram".to_owned(),
        }
        .into()),
    }
}

#[derive(Debug)]
pub struct BankingPreLayoutLowering {
    pub default_policy: InterruptPolicy,
    pub default_lifetime: LeaseLifetime,
    pub assert_bank_policy: BankingAssertBankPolicy,
    pub safety_table: InterruptSafetyTable,
    pub residency_table: BTreeMap<SectionId, SectionResidency>,
    trust_token: BankingLoweringToken,
    state: RefCell<BankingLoweringState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BankingLoweringToken(u64);

impl BankingLoweringToken {
    fn new() -> Self {
        Self(NEXT_LOWERING_TOKEN.fetch_add(1, Ordering::Relaxed))
    }

    fn note(self) -> alloc::string::String {
        alloc::format!("{BANKING_PROVENANCE_NOTE_PREFIX}:{:016x}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BankingAssertBankPolicy {
    LabelOnly,
    CompareAndTrap,
}

impl BankingPreLayoutLowering {
    #[must_use]
    pub fn new(
        default_policy: InterruptPolicy,
        default_lifetime: LeaseLifetime,
        safety_table: InterruptSafetyTable,
    ) -> Self {
        Self {
            default_policy,
            default_lifetime,
            assert_bank_policy: BankingAssertBankPolicy::LabelOnly,
            safety_table,
            residency_table: BTreeMap::new(),
            trust_token: BankingLoweringToken::new(),
            state: RefCell::new(BankingLoweringState::default()),
        }
    }

    #[must_use]
    pub fn with_residency(mut self, section: SectionId, residency: SectionResidency) -> Self {
        self.residency_table.insert(section, residency);
        self
    }

    #[must_use]
    pub fn with_assert_bank_policy(mut self, policy: BankingAssertBankPolicy) -> Self {
        self.assert_bank_policy = policy;
        self
    }

    pub(crate) fn lower_with_state(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
        state: &mut BankingLoweringState,
    ) -> LoweringDisposition {
        match op {
            PreLayoutOp::BankLease(spec) => self.lower_bank_lease(spec, ctx, state).map_or_else(
                |err| LoweringDisposition::Error(LoweringError::Runtime(err.to_string())),
                LoweringDisposition::Lowered,
            ),
            PreLayoutOp::BankRelease {
                lease_id,
                return_to,
            } => self
                .lower_bank_release(*lease_id, *return_to, ctx, state)
                .map_or_else(
                    |err| LoweringDisposition::Error(LoweringError::Runtime(err.to_string())),
                    LoweringDisposition::Lowered,
                ),
            PreLayoutOp::AssertBank {
                expected,
                expected_n,
            } => self
                .lower_assert_bank(*expected, *expected_n, ctx)
                .map_or_else(
                    |err| LoweringDisposition::Error(LoweringError::Runtime(err.to_string())),
                    LoweringDisposition::Lowered,
                ),
            PreLayoutOp::Yield { .. } | PreLayoutOp::TraceProbe { .. } => {
                LoweringDisposition::NotOwned
            }
        }
    }

    fn lower_bank_lease(
        &self,
        spec: &BankLeaseSpec,
        ctx: &LoweringContext<'_>,
        state: &mut BankingLoweringState,
    ) -> Result<LoweredFragment, BankingEmitError> {
        self.check_ctx(ctx)?;
        let lifetime = spec.lifetime();
        if matches!(lifetime, LeaseLifetime::ResumeWindow | LeaseLifetime::Token) {
            return Err(BankAbiViolation::UnsupportedRestoringLifetime { lifetime }.into());
        }
        match spec.class() {
            MbcBankClass::Rom => {
                ValidatedBankLeaseSpec::for_rom_switchable(spec.bank(), lifetime)?;
            }
            MbcBankClass::Sram => {
                ValidatedBankLeaseSpec::for_sram_bank(spec.bank(), lifetime)?;
            }
        }
        if let Some(existing) = state.active_lease_id() {
            return Err(BankAbiViolation::NestedLeaseNotSupported {
                class: spec.class(),
                existing,
                attempted: spec.lease_id(),
            }
            .into());
        }

        let lease = BankLease {
            id: spec.lease_id(),
            spec: spec.clone(),
            lifetime,
        };
        let mut b = fragment_builder(ctx, "lease")?;
        b.with_provenance(banking_provenance(ctx.provenance, self.trust_token), |b| {
            match spec.class() {
                MbcBankClass::Rom => {
                    lower_acquire_rom_bank(b, state, spec.bank(), self.default_policy)?;
                }
                MbcBankClass::Sram => {
                    lower_enable_sram(b, state, self.default_policy)?;
                    lower_acquire_sram_bank(b, state, spec.bank() as u8, self.default_policy)?;
                }
            }
            Ok::<(), BankingEmitError>(())
        })?;
        state.active_leases.insert(lease.id, ActiveLease { lease });
        fragment_from_builder(b)
    }

    fn lower_bank_release(
        &self,
        lease_id: LeaseId,
        return_to: BankReleaseDisposition,
        ctx: &LoweringContext<'_>,
        state: &mut BankingLoweringState,
    ) -> Result<LoweredFragment, BankingEmitError> {
        self.check_ctx(ctx)?;
        let active = state
            .active_leases
            .remove(&lease_id)
            .ok_or(BankAbiViolation::UnreleasedLease { lease: lease_id })?;
        if return_to == BankReleaseDisposition::KeepCurrent {
            return Err(BankAbiViolation::KeepCurrentProofMissing { lease: lease_id }.into());
        }
        let mut b = fragment_builder(ctx, "release")?;
        b.with_provenance(banking_provenance(ctx.provenance, self.trust_token), |b| {
            lower_release(b, state, &active.lease, return_to, self.default_policy)
        })?;
        fragment_from_builder(b)
    }

    fn lower_assert_bank(
        &self,
        expected: MbcBankClass,
        expected_n: u16,
        ctx: &LoweringContext<'_>,
    ) -> Result<LoweredFragment, BankingEmitError> {
        let mut b = fragment_builder(ctx, "assert_bank")?;
        let label = banking_label(&format!(
            "assert_{}_{}_s{}",
            bank_class_segment(expected),
            expected_n,
            ctx.source_section_id.get()
        ))?;
        b.with_provenance(banking_provenance(ctx.provenance, self.trust_token), |b| {
            b.label(label);
            if self.assert_bank_policy == BankingAssertBankPolicy::CompareAndTrap {
                emit_assert_bank_compare_and_trap(b, expected, expected_n)?;
            }
            Ok::<(), BankingEmitError>(())
        })?;
        fragment_from_builder(b)
    }

    fn check_ctx(&self, ctx: &LoweringContext<'_>) -> Result<(), BankingEmitError> {
        let safety = self
            .safety_table
            .lookup(ctx.source_section_id)
            .unwrap_or(InterruptSafety {
                kind: InterruptSafetyKind::InterruptDisabled,
            });
        let residency = self
            .residency_table
            .get(&ctx.source_section_id)
            .copied()
            .unwrap_or_else(|| SectionResidency::from_section_role(ctx.source_section_role));
        check_lease_emission_legal_parts(
            ctx.source_section_id,
            ctx.source_section_privilege,
            safety,
            residency,
        )?;
        Ok(())
    }
}

impl Default for BankingPreLayoutLowering {
    fn default() -> Self {
        Self::new(
            InterruptPolicy::ShortCriticalSection,
            LeaseLifetime::Slice,
            InterruptSafetyTable::default(),
        )
    }
}

impl PreLayoutOpLowering for BankingPreLayoutLowering {
    fn lower(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
    ) -> Result<LoweredFragment, LoweringError> {
        let mut state = self.state.borrow_mut();
        match self.lower_with_state(op, ctx, &mut state) {
            LoweringDisposition::Lowered(fragment) => Ok(fragment),
            LoweringDisposition::NotOwned => {
                Err(LoweringError::UnsupportedStructuredOp(op_name(op)))
            }
            LoweringDisposition::Error(err) => Err(err),
        }
    }
}

impl DispositionPreLayoutOpLowering for BankingPreLayoutLowering {
    fn lower_disposition(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
    ) -> LoweringDisposition {
        let mut state = self.state.borrow_mut();
        self.lower_with_state(op, ctx, &mut state)
    }
}

fn emit_assert_bank_compare_and_trap(
    b: &mut Builder,
    expected: MbcBankClass,
    expected_n: u16,
) -> Result<(), BankingEmitError> {
    match expected {
        MbcBankClass::Rom => {
            if expected_n == 0 {
                return Err(BankAbiViolation::RomBankZeroReservedByAbi.into());
            }
            if expected_n > BankLeaseSpec::MAX_ROM_BANK {
                return Err(BankAbiViolation::RomBankOutOfRange { bank: expected_n }.into());
            }
            let lo = expected_n as u8;
            let hi = ((expected_n >> 8) & 0x01) as u8;
            b.try_emit(Instr::LdAFromHighDirect {
                offset: HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_LO),
            })?;
            b.try_emit(Instr::CpA {
                src: AluSrc8::Imm(lo),
            })?;
            b.try_emit(Instr::JrRel {
                cond: Some(Cond::NZ),
                off: 6,
            })?;
            b.try_emit(Instr::LdAFromHighDirect {
                offset: HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_HI),
            })?;
            b.try_emit(Instr::CpA {
                src: AluSrc8::Imm(hi),
            })?;
            b.try_emit(Instr::JrRel {
                cond: Some(Cond::NZ),
                off: 0,
            })?;
        }
        MbcBankClass::Sram => {
            validate_sram_bank(expected_n)?;
            b.try_emit(Instr::LdAFromHighDirect {
                offset: HighDirectOffset::new(HRAM_LDH_CURRENT_SRAM_BANK),
            })?;
            b.try_emit(Instr::CpA {
                src: AluSrc8::Imm(expected_n as u8),
            })?;
            b.try_emit(Instr::JrRel {
                cond: Some(Cond::NZ),
                off: 0,
            })?;
        }
    }
    b.try_emit(Instr::Rst {
        vector: RstVector::V38,
    })?;
    Ok(())
}

pub fn mbc_write_provenance_audit(
    sections: &[gbf_asm::section::LoweredSection],
    safety_table: &InterruptSafetyTable,
    lowerer: &BankingPreLayoutLowering,
) -> Result<(), BankAbiViolation> {
    for section in sections {
        let safety = safety_table.lookup(section.id).unwrap_or(InterruptSafety {
            kind: InterruptSafetyKind::InterruptDisabled,
        });
        let residency = SectionResidency::from_section_role(section.role);
        for item in &section.instrs {
            if matches!(
                classify_effect(&item.data),
                MachineEffect::StoreToMbcRegister { .. }
            ) {
                check_lease_emission_legal_parts(
                    section.id,
                    &section.privilege,
                    safety,
                    residency,
                )?;
                if !is_trusted_banking_op(&item.provenance, lowerer.trust_token) {
                    return Err(BankAbiViolation::MbcWriteOutsideBankingProvenance {
                        section: section.id,
                    });
                }
            }
        }
    }
    Ok(())
}

fn fragment_builder(ctx: &LoweringContext<'_>, symbol: &str) -> Result<Builder, BankingEmitError> {
    let name = banking_label(&format!("{symbol}_s{}", ctx.source_section_id.get()))?;
    Ok(
        Builder::new_with_id(ctx.source_section_id, ctx.source_section_role, name)
            .with_section_privilege(SectionPrivilege::privileged()),
    )
}

fn fragment_from_builder(builder: Builder) -> Result<LoweredFragment, BankingEmitError> {
    let section = builder.try_finish()?;
    let mut fragment = LoweredFragment::default();
    for item in section.labels() {
        fragment.labels.push(FragmentItem::new_with_sub_index(
            item.data.clone(),
            checked_sub_index(item.seq_index)?,
            item.provenance.clone(),
        ));
    }
    for item in section.instrs() {
        fragment.instrs.push(FragmentItem::new_with_sub_index(
            item.data,
            checked_sub_index(item.seq_index)?,
            item.provenance.clone(),
        ));
    }
    for item in section.data_blocks() {
        fragment.data_blocks.push(FragmentItem::new_with_sub_index(
            item.data.clone(),
            checked_sub_index(item.seq_index)?,
            item.provenance.clone(),
        ));
    }
    for item in section.alignments() {
        fragment.alignments.push(FragmentItem::new_with_sub_index(
            item.data,
            checked_sub_index(item.seq_index)?,
            item.provenance.clone(),
        ));
    }
    for item in section.legalization_ops() {
        fragment
            .legalization_ops
            .push(FragmentItem::new_with_sub_index(
                item.data.clone(),
                checked_sub_index(item.seq_index)?,
                item.provenance.clone(),
            ));
    }
    for item in section.branches() {
        fragment.branches.push(FragmentItem::new_with_sub_index(
            item.data.clone(),
            checked_sub_index(item.seq_index)?,
            item.provenance.clone(),
        ));
    }
    Ok(fragment)
}

fn checked_sub_index(seq_index: u32) -> Result<u16, BankingEmitError> {
    u16::try_from(seq_index)
        .map_err(|_| BankingEmitError::Builder("lowered fragment sub-index overflow".into()))
}

fn emit_release_label(b: &mut Builder, lease: LeaseId) -> Result<(), BankingEmitError> {
    let label = banking_label(&format!("release_keep_current_{}", lease.get()))?;
    b.with_provenance(untrusted_banking_provenance(b.current_provenance()), |b| {
        b.label(label);
    });
    Ok(())
}

fn validate_banking_shadow_offset(offset: HighDirectOffset) -> Result<(), BankingEmitError> {
    match offset.get() {
        HRAM_LDH_CURRENT_ROM_BANK_LO
        | HRAM_LDH_CURRENT_ROM_BANK_HI
        | HRAM_LDH_CURRENT_SRAM_BANK
        | HRAM_LDH_SRAM_ENABLED => Ok(()),
        other => Err(BankingEmitError::ShadowOffsetOutOfRange { offset: other }),
    }
}

fn validate_return_state_class(
    lease_class: MbcBankClass,
    return_state: ReturnState,
) -> Result<(), BankAbiViolation> {
    match (lease_class, return_state) {
        (MbcBankClass::Rom, ReturnState::Rom(_) | ReturnState::KeepCurrent(_)) => Ok(()),
        (MbcBankClass::Sram, ReturnState::Sram(_) | ReturnState::KeepCurrent(_)) => Ok(()),
        (lease_class, return_state) => Err(BankAbiViolation::ReturnStateWrongClass {
            lease_class,
            return_class: return_state.return_class_name().to_owned(),
        }),
    }
}

fn validate_return_rom_bank(bank: u16) -> Result<(), BankAbiViolation> {
    if bank == 0 || bank > BankLeaseSpec::MAX_ROM_BANK {
        Err(BankAbiViolation::ReturnBankOutOfRange { bank })
    } else {
        Ok(())
    }
}

fn validate_sram_bank(bank: u16) -> Result<(), BankAbiViolation> {
    if bank > BankLeaseSpec::MAX_SRAM_BANK {
        Err(BankAbiViolation::SramBankOutOfRange { bank })
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum BankingPolicyOp {
    RomAcquire,
    SramAcquire,
    SramEnable,
    SramDisable,
}

fn guard_policy(policy: InterruptPolicy, op: BankingPolicyOp) -> Result<(), BankingEmitError> {
    if policy != InterruptPolicy::Enabled {
        return Ok(());
    }
    Err(match op {
        BankingPolicyOp::RomAcquire => BankingEmitError::EnabledPolicyForRomAcquire,
        BankingPolicyOp::SramAcquire => BankingEmitError::EnabledPolicyForSramAcquire,
        BankingPolicyOp::SramEnable => BankingEmitError::EnabledPolicyForSramEnable,
        BankingPolicyOp::SramDisable => BankingEmitError::EnabledPolicyForSramDisable,
    })
}

fn emit_policy_prologue(b: &mut Builder, policy: InterruptPolicy) -> Result<(), BankingEmitError> {
    if policy == InterruptPolicy::ShortCriticalSection {
        b.try_emit(Instr::Di)?;
    }
    Ok(())
}

fn emit_policy_epilogue(b: &mut Builder, policy: InterruptPolicy) -> Result<(), BankingEmitError> {
    if policy == InterruptPolicy::ShortCriticalSection {
        b.try_emit(Instr::Ei)?;
    }
    Ok(())
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("MBC register addresses are below $FF00")
}

fn untrusted_banking_provenance(provenance: &InstrProvenance) -> InstrProvenance {
    provenance.clone().with_source_op(BANKING_SOURCE_OP)
}

fn banking_provenance(
    provenance: &InstrProvenance,
    token: BankingLoweringToken,
) -> InstrProvenance {
    provenance
        .clone()
        .with_source_op(BANKING_SOURCE_OP)
        .with_note(token.note())
}

fn is_trusted_banking_op(provenance: &InstrProvenance, token: BankingLoweringToken) -> bool {
    let source_matches = provenance
        .source_op
        .as_ref()
        .is_some_and(|source| source.as_ref() == BANKING_SOURCE_OP);
    let note_matches = provenance
        .note
        .as_ref()
        .is_some_and(|note| note.as_ref() == token.note());
    source_matches && note_matches
}

fn banking_label(segment: &str) -> Result<SymbolName, BankingEmitError> {
    SymbolName::runtime("banking", segment)
        .map_err(|err| BankingEmitError::SymbolName(err.to_string()))
}

const fn bank_class_segment(class: MbcBankClass) -> &'static str {
    match class {
        MbcBankClass::Rom => "rom",
        MbcBankClass::Sram => "sram",
    }
}

const fn op_name(op: &PreLayoutOp) -> &'static str {
    match op {
        PreLayoutOp::BankLease(_) => "BankLease",
        PreLayoutOp::BankRelease { .. } => "BankRelease",
        PreLayoutOp::Yield { .. } => "Yield",
        PreLayoutOp::TraceProbe { .. } => "TraceProbe",
        PreLayoutOp::AssertBank { .. } => "AssertBank",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_asm::encoder::encode_instr;
    use gbf_asm::lowering::{lower_pre_layout_ops, lower_pre_layout_ops_with_disposition};
    use gbf_asm::provenance::PlanningStage;
    use gbf_asm::section::{LoweredSection, OrderedItem, SectionPrivilege, YieldKind};
    use gbf_asm::symbols::SymbolTable;

    fn banking_builder(role: SectionRole) -> Builder {
        Builder::new(
            role,
            SymbolName::runtime("banking", "test").expect("section name"),
        )
        .with_section_privilege(SectionPrivilege::privileged())
    }

    fn test_bank_lease_spec(
        lease: LeaseId,
        class: MbcBankClass,
        bank: u16,
    ) -> Result<BankLeaseSpec, gbf_asm::section::BankLeaseSpecError> {
        BankLeaseSpec::new(
            lease,
            LeaseGeneration(lease.get()),
            class,
            bank,
            LeaseLifetime::Slice,
        )
    }

    fn test_bank_lease_spec_with_lifetime(
        lease: LeaseId,
        class: MbcBankClass,
        bank: u16,
        lifetime: LeaseLifetime,
    ) -> Result<BankLeaseSpec, gbf_asm::section::BankLeaseSpecError> {
        BankLeaseSpec::new(lease, LeaseGeneration(lease.get()), class, bank, lifetime)
    }

    fn instr_bytes(section: &Section) -> Vec<u8> {
        section
            .instrs()
            .iter()
            .flat_map(|item| encode_instr(&item.data).expect("instr encodes"))
            .collect()
    }

    fn fragment_bytes(fragment: &LoweredFragment) -> Vec<u8> {
        fragment
            .instrs
            .iter()
            .flat_map(|item| encode_instr(&item.data).expect("instr encodes"))
            .collect()
    }

    fn instr_cycles(section: &Section) -> u32 {
        section
            .instrs()
            .iter()
            .map(|item| u32::from(item.data.cycle_cost().worst_case()))
            .sum()
    }

    fn lowerer_ctx<'a>(
        section_id: SectionId,
        role: SectionRole,
        privilege: &'a SectionPrivilege,
        provenance: &'a InstrProvenance,
        symbols: &'a SymbolTable,
    ) -> LoweringContext<'a> {
        LoweringContext {
            source_section_id: section_id,
            source_section_role: role,
            source_section_privilege: privilege,
            provenance,
            symbols,
        }
    }

    #[test]
    fn lease_spec_invariants_rom() {
        assert_eq!(
            ValidatedBankLeaseSpec::for_rom_switchable(0, LeaseLifetime::Slice),
            Err(BankAbiViolation::RomBankZeroReservedByAbi)
        );
        assert!(ValidatedBankLeaseSpec::for_rom_switchable(1, LeaseLifetime::Slice).is_ok());
        assert!(ValidatedBankLeaseSpec::for_rom_switchable(511, LeaseLifetime::Slice).is_ok());
        assert_eq!(
            ValidatedBankLeaseSpec::for_rom_switchable(512, LeaseLifetime::Slice),
            Err(BankAbiViolation::RomBankOutOfRange { bank: 512 })
        );
    }

    #[test]
    fn lease_spec_invariants_sram() {
        assert!(ValidatedBankLeaseSpec::for_sram(0, LeaseLifetime::Slice).is_ok());
        assert!(ValidatedBankLeaseSpec::for_sram(15, LeaseLifetime::Slice).is_ok());
        assert_eq!(
            ValidatedBankLeaseSpec::for_sram_bank(16, LeaseLifetime::Slice),
            Err(BankAbiViolation::SramBankOutOfRange { bank: 16 })
        );
    }

    #[test]
    fn lifetime_yield_safety() {
        assert!(LeaseLifetime::Slice.yield_safe());
        assert!(LeaseLifetime::ResumeWindow.yield_safe());
        assert!(LeaseLifetime::Token.yield_safe());
        assert!(!LeaseLifetime::Manual.yield_safe());
    }

    #[test]
    fn bank_guard_does_not_borrow_builder() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        builder.emit(Instr::Nop);
        release_bank(&mut builder, guard, ReturnState::Rom(ReturnRomBank::Bank1)).expect("release");
        assert_eq!(builder.finish().total_items(), 3);
    }

    #[test]
    fn bank_guard_drop_without_release() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let _guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        let err = builder.try_finish().expect_err("unreleased guard rejected");
        assert!(matches!(err, BuilderError::UnreleasedBankGuard { .. }));
    }

    #[test]
    fn bank_guard_double_release() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        let stale = BankGuard {
            lease_id: guard.lease_id(),
            source_section_id: guard.source_section_id(),
            generation: guard.generation(),
            class: guard.class(),
            bank: guard.bank(),
            lifetime: guard.lifetime(),
        };
        let stale_lease = stale.lease_id();
        release_bank(&mut builder, guard, ReturnState::Rom(ReturnRomBank::Bank1))
            .expect("first release");
        assert_eq!(
            release_bank(&mut builder, stale, ReturnState::Rom(ReturnRomBank::Bank1)),
            Err(BankingEmitError::Abi(BankAbiViolation::StaleBankGuard {
                lease: stale_lease
            }))
        );
    }

    #[test]
    fn bank_guard_from_other_builder_is_stale() {
        let mut source = banking_builder(SectionRole::Bank0Nucleus);
        let guard = lease_rom_switchable(
            &mut source,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");

        let mut other = banking_builder(SectionRole::Bank0Nucleus);
        let _other_guard = lease_rom_switchable(
            &mut other,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("other lease");

        assert_eq!(
            release_bank(&mut other, guard, ReturnState::Rom(ReturnRomBank::Bank1)),
            Err(BankingEmitError::Abi(BankAbiViolation::StaleBankGuard {
                lease: LeaseId::new(0)
            }))
        );
    }

    #[test]
    fn return_state_class_correct() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        assert_eq!(
            release_bank(
                &mut builder,
                guard,
                ReturnState::Sram(ReturnSramState::Disable)
            ),
            Err(BankingEmitError::Abi(
                BankAbiViolation::ReturnStateWrongClass {
                    lease_class: MbcBankClass::Rom,
                    return_class: "sram".to_owned(),
                }
            ))
        );
    }

    #[test]
    fn return_bank_zero_rejected() {
        assert_eq!(
            ReturnState::Rom(ReturnRomBank::Manual(0)).disposition(),
            Err(BankAbiViolation::ReturnBankOutOfRange { bank: 0 })
        );
    }

    #[test]
    fn manual_lease_rejects_yield_while_held() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let _guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(2, LeaseLifetime::Manual).expect("spec"),
        )
        .expect("lease");
        assert!(matches!(
            builder.try_yield_op(YieldKind::Cooperative),
            Err(BuilderError::YieldWithActiveLease { .. })
        ));
    }

    #[test]
    fn nested_rom_lease_rejected_or_stack_restored() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let _guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(2, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        assert!(matches!(
            lease_rom_switchable(
                &mut builder,
                ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
            ),
            Err(BankingEmitError::Abi(
                BankAbiViolation::NestedLeaseNotSupported { .. }
            ))
        ));
    }

    #[test]
    fn nested_sram_lease_rejected_or_stack_restored() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let _guard = lease_sram(
            &mut builder,
            ValidatedBankLeaseSpec::for_sram(2, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        assert!(matches!(
            lease_sram(
                &mut builder,
                ValidatedBankLeaseSpec::for_sram(3, LeaseLifetime::Slice).expect("spec"),
            ),
            Err(BankingEmitError::Abi(
                BankAbiViolation::NestedLeaseNotSupported { .. }
            ))
        ));
    }

    #[test]
    fn hram_shadow_offsets_within_banking_range() {
        assert_eq!(HRAM_ADDR_CURRENT_ROM_BANK_LO, 0xFF80);
        assert_eq!(HRAM_ADDR_CURRENT_ROM_BANK_HI, 0xFF81);
        assert_eq!(HRAM_ADDR_CURRENT_SRAM_BANK, 0xFF82);
        assert_eq!(HRAM_ADDR_SRAM_ENABLED, 0xFF83);
        for addr in [
            HRAM_ADDR_CURRENT_ROM_BANK_LO,
            HRAM_ADDR_CURRENT_ROM_BANK_HI,
            HRAM_ADDR_CURRENT_SRAM_BANK,
            HRAM_ADDR_SRAM_ENABLED,
        ] {
            assert!(memory::is_hram(addr));
        }
    }

    #[test]
    fn hram_banking_shadow_region_size() {
        assert_eq!(HRAM_BANKING_SHADOW_END_EXCLUSIVE - HRAM_SHADOW_BASE, 4);
    }

    #[test]
    fn store_bank_shadow_emits_ldh() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        emit_store_bank_shadow_byte_imm(
            &mut builder,
            HighDirectOffset::new(HRAM_LDH_CURRENT_ROM_BANK_LO),
            0x12,
        )
        .expect("store");
        assert_eq!(instr_bytes(&builder.finish()), vec![0x3E, 0x12, 0xE0, 0x80]);
    }

    #[test]
    fn store_bank_shadow_rejects_non_banking_offsets() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        assert_eq!(
            emit_store_bank_shadow_byte_imm(&mut builder, HighDirectOffset::new(0x84), 0),
            Err(BankingEmitError::ShadowOffsetOutOfRange { offset: 0x84 })
        );
    }

    #[test]
    fn banking_shadow_zero_init_byte_and_cycle_count() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        lower_banking_shadow_zero_init(&mut builder).expect("zero init");
        let section = builder.finish();
        assert_eq!(
            instr_bytes(&section),
            vec![0x3E, 0x00, 0xE0, 0x80, 0xE0, 0x81, 0xE0, 0x82, 0xE0, 0x83]
        );
        assert_eq!(instr_cycles(&section), 14);
    }

    #[test]
    fn lower_enable_sram_byte_sequence() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        lower_enable_sram(
            &mut builder,
            &mut state,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("enable");
        assert_eq!(
            instr_bytes(&builder.finish()),
            vec![0xF3, 0x3E, 0x0A, 0xEA, 0x00, 0x00, 0xE0, 0x83, 0xFB]
        );
        assert!(state.sram_enabled);
    }

    #[test]
    fn lower_disable_sram_byte_sequence() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState {
            sram_enabled: true,
            current_sram_bank: Some(2),
            ..BankingLoweringState::default()
        };
        lower_disable_sram(
            &mut builder,
            &mut state,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("disable");
        assert_eq!(
            instr_bytes(&builder.finish()),
            vec![0xF3, 0x3E, 0x00, 0xEA, 0x00, 0x00, 0xE0, 0x83, 0xFB]
        );
        assert!(!state.sram_enabled);
    }

    #[test]
    fn lower_acquire_rom_bank_byte_sequence_bank3() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        lower_acquire_rom_bank(
            &mut builder,
            &mut state,
            3,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("acquire");
        let section = builder.finish();
        assert_eq!(
            instr_bytes(&section),
            vec![
                0xF3, 0x3E, 0x03, 0xEA, 0x00, 0x20, 0x3E, 0x00, 0xEA, 0x00, 0x30, 0x3E, 0x03, 0xE0,
                0x80, 0x3E, 0x00, 0xE0, 0x81, 0xFB,
            ]
        );
        assert_eq!(instr_cycles(&section), 24);
    }

    #[test]
    fn lower_acquire_rom_bank_byte_sequence_bank256() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        lower_acquire_rom_bank(
            &mut builder,
            &mut state,
            256,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("acquire");
        assert_eq!(
            instr_bytes(&builder.finish()),
            vec![
                0xF3, 0x3E, 0x00, 0xEA, 0x00, 0x20, 0x3E, 0x01, 0xEA, 0x00, 0x30, 0x3E, 0x00, 0xE0,
                0x80, 0x3E, 0x01, 0xE0, 0x81, 0xFB,
            ]
        );
    }

    #[test]
    fn lower_acquire_rom_bank_disabled_policy() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        lower_acquire_rom_bank(&mut builder, &mut state, 3, InterruptPolicy::Disabled)
            .expect("acquire");
        let section = builder.finish();
        assert_eq!(instr_bytes(&section).len(), 18);
        assert_eq!(instr_cycles(&section), 22);
    }

    #[test]
    fn lower_acquire_rom_bank_rejects_enabled_policy() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        assert_eq!(
            lower_acquire_rom_bank(&mut builder, &mut state, 3, InterruptPolicy::Enabled),
            Err(BankingEmitError::EnabledPolicyForRomAcquire)
        );
    }

    #[test]
    fn lower_acquire_rom_bank_rejects_bank_zero() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        assert_eq!(
            lower_acquire_rom_bank(
                &mut builder,
                &mut state,
                0,
                InterruptPolicy::ShortCriticalSection
            ),
            Err(BankingEmitError::Abi(
                BankAbiViolation::RomBankZeroReservedByAbi
            ))
        );
    }

    #[test]
    fn lower_acquire_sram_bank_byte_sequence() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState {
            sram_enabled: true,
            ..BankingLoweringState::default()
        };
        lower_acquire_sram_bank(
            &mut builder,
            &mut state,
            5,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("acquire");
        assert_eq!(
            instr_bytes(&builder.finish()),
            vec![0xF3, 0x3E, 0x05, 0xEA, 0x00, 0x40, 0xE0, 0x82, 0xFB]
        );
    }

    #[test]
    fn lower_acquire_sram_bank_rejects_disabled() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        assert_eq!(
            lower_acquire_sram_bank(
                &mut builder,
                &mut state,
                5,
                InterruptPolicy::ShortCriticalSection
            ),
            Err(BankingEmitError::Abi(
                BankAbiViolation::SramBankAcquiredWhileDisabled
            ))
        );
    }

    #[test]
    fn byte_stable_emit_property() {
        for policy in [
            InterruptPolicy::ShortCriticalSection,
            InterruptPolicy::Disabled,
        ] {
            for bank in [1, 3, 255, 256, 511] {
                let mut left = banking_builder(SectionRole::Bank0Nucleus);
                let mut right = banking_builder(SectionRole::Bank0Nucleus);
                let mut left_state = BankingLoweringState::default();
                let mut right_state = BankingLoweringState::default();
                lower_acquire_rom_bank(&mut left, &mut left_state, bank, policy).expect("left");
                lower_acquire_rom_bank(&mut right, &mut right_state, bank, policy).expect("right");
                assert_eq!(instr_bytes(&left.finish()), instr_bytes(&right.finish()));
            }

            for bank in [0, 5, 15] {
                let mut left = banking_builder(SectionRole::Bank0Nucleus);
                let mut right = banking_builder(SectionRole::Bank0Nucleus);
                let mut left_state = BankingLoweringState {
                    sram_enabled: true,
                    ..BankingLoweringState::default()
                };
                let mut right_state = BankingLoweringState {
                    sram_enabled: true,
                    ..BankingLoweringState::default()
                };
                lower_acquire_sram_bank(&mut left, &mut left_state, bank, policy).expect("left");
                lower_acquire_sram_bank(&mut right, &mut right_state, bank, policy).expect("right");
                assert_eq!(instr_bytes(&left.finish()), instr_bytes(&right.finish()));
            }
        }
    }

    #[test]
    fn lower_release_keep_current_is_label_only() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        let lease = BankLease {
            id: LeaseId::new(9),
            spec: test_bank_lease_spec(LeaseId::new(9), MbcBankClass::Rom, 3).expect("spec"),
            lifetime: LeaseLifetime::Slice,
        };
        lower_release(
            &mut builder,
            &mut state,
            &lease,
            BankReleaseDisposition::KeepCurrent,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("release");
        let section = builder.finish();
        assert_eq!(section.instrs().len(), 0);
        assert_eq!(section.labels().len(), 1);
    }

    #[test]
    fn lower_release_to_bank1_is_acquire_one() {
        let mut release_builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut acquire_builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut release_state = BankingLoweringState::default();
        let mut acquire_state = BankingLoweringState::default();
        let lease = BankLease {
            id: LeaseId::new(9),
            spec: test_bank_lease_spec(LeaseId::new(9), MbcBankClass::Rom, 3).expect("spec"),
            lifetime: LeaseLifetime::Slice,
        };
        lower_release(
            &mut release_builder,
            &mut release_state,
            &lease,
            BankReleaseDisposition::RomBank1,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("release");
        lower_acquire_rom_bank(
            &mut acquire_builder,
            &mut acquire_state,
            1,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("acquire");
        assert_eq!(
            instr_bytes(&release_builder.finish()),
            instr_bytes(&acquire_builder.finish())
        );
    }

    #[test]
    fn all_emits_are_privileged() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let mut state = BankingLoweringState::default();
        lower_acquire_rom_bank(
            &mut builder,
            &mut state,
            3,
            InterruptPolicy::ShortCriticalSection,
        )
        .expect("acquire");
        let mbc_effects = builder
            .finish()
            .instrs()
            .iter()
            .filter_map(|item| match classify_effect(&item.data) {
                MachineEffect::StoreToMbcRegister { reg } => Some(reg),
                _ => None,
            })
            .count();
        assert_eq!(mbc_effects, 2);
    }

    #[test]
    fn isr_section_cannot_lease() {
        let section = Section::new(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "isr").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        )
        .with_privilege(SectionPrivilege::privileged());
        assert_eq!(
            check_lease_emission_legal(
                &section,
                InterruptSafety {
                    kind: InterruptSafetyKind::InterruptHandler
                },
                SectionResidency::FixedRom0,
            ),
            Err(BankAbiViolation::IsrCannotAcquire {
                section: SectionId::new(1)
            })
        );
    }

    #[test]
    fn privileged_non_isr_fixed_section_can_lease() {
        let section = Section::new(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "ok").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        )
        .with_privilege(SectionPrivilege::privileged());
        assert!(
            check_lease_emission_legal(
                &section,
                InterruptSafety {
                    kind: InterruptSafetyKind::InterruptDisabled
                },
                SectionResidency::FixedRom0,
            )
            .is_ok()
        );
    }

    #[test]
    fn normal_section_cannot_lease() {
        let section = Section::new(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "normal").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        );
        assert!(matches!(
            check_lease_emission_legal(
                &section,
                InterruptSafety {
                    kind: InterruptSafetyKind::InterruptDisabled
                },
                SectionResidency::FixedRom0,
            ),
            Err(BankAbiViolation::SectionNotPrivileged { .. })
        ));
    }

    #[test]
    fn switchable_residency_cannot_lease() {
        let section = Section::new(
            SectionId::new(1),
            SectionRole::CommonBank,
            SymbolName::runtime("banking", "switchable").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        )
        .with_privilege(SectionPrivilege::privileged());
        assert!(matches!(
            check_lease_emission_legal(
                &section,
                InterruptSafety {
                    kind: InterruptSafetyKind::InterruptDisabled
                },
                SectionResidency::SwitchableRom,
            ),
            Err(BankAbiViolation::BankingPrimitiveNotFixedResident { .. })
        ));
    }

    #[test]
    fn annotation_serializable_round_trip() {
        let section = Section::new(
            SectionId::new(7),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "annotated").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        );
        let mut table = InterruptSafetyTable::default();
        mark_isr_reachable(&mut table, &section).expect("mark");
        let exported = table.export();
        let decoded: InterruptSafetyTable =
            serde_json::from_value(exported).expect("table deserializes");
        assert_eq!(decoded, table);
    }

    #[test]
    fn declare_conflicting_kind_errors() {
        let section = Section::new(
            SectionId::new(7),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "conflict").expect("name"),
            std::num::NonZeroU16::new(1).expect("nonzero"),
        );
        let mut table = InterruptSafetyTable::default();
        mark_isr_reachable(&mut table, &section).expect("first");
        mark_isr_reachable(&mut table, &section).expect("same kind ok");
        assert!(matches!(
            mark_isr(&mut table, &section),
            Err(InterruptSafetyError::ConflictingDeclaration { .. })
        ));
    }

    #[test]
    fn lowering_bank_lease_emits_acquire_sequence() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let spec = test_bank_lease_spec(LeaseId::new(1), MbcBankClass::Rom, 3).expect("spec");
        let fragment =
            match lowerer.lower_with_state(&PreLayoutOp::BankLease(spec), &ctx, &mut state) {
                LoweringDisposition::Lowered(fragment) => fragment,
                other => panic!("unexpected lowering disposition: {other:?}"),
            };
        assert_eq!(fragment_bytes(&fragment).len(), 20);
    }

    #[test]
    fn lowering_preserves_manual_lifetime_from_wire_spec() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let spec = test_bank_lease_spec_with_lifetime(
            LeaseId::new(1),
            MbcBankClass::Rom,
            3,
            LeaseLifetime::Manual,
        )
        .expect("spec");
        assert!(matches!(
            lowerer.lower_with_state(&PreLayoutOp::BankLease(spec), &ctx, &mut state),
            LoweringDisposition::Lowered(_)
        ));
        let active = state
            .active_leases
            .get(&LeaseId::new(1))
            .expect("active lease");
        assert_eq!(active.lease.lifetime, LeaseLifetime::Manual);
    }

    #[test]
    fn lowering_rejects_restoration_lifetimes_until_scheduler_owner_lands() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        for (idx, lifetime) in [LeaseLifetime::ResumeWindow, LeaseLifetime::Token]
            .into_iter()
            .enumerate()
        {
            let spec = test_bank_lease_spec_with_lifetime(
                LeaseId::new(idx as u32 + 1),
                MbcBankClass::Rom,
                3,
                lifetime,
            )
            .expect("spec");
            assert!(matches!(
                lowerer.lower_with_state(&PreLayoutOp::BankLease(spec), &ctx, &mut state),
                LoweringDisposition::Error(LoweringError::Runtime(message))
                    if message.contains("scheduler restoration")
            ));
        }
    }

    #[test]
    fn lowering_bank_release_per_return_state() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let spec = test_bank_lease_spec(LeaseId::new(1), MbcBankClass::Rom, 3).expect("spec");
        assert!(matches!(
            lowerer.lower_with_state(&PreLayoutOp::BankLease(spec), &ctx, &mut state),
            LoweringDisposition::Lowered(_)
        ));
        let fragment = match lowerer.lower_with_state(
            &PreLayoutOp::BankRelease {
                lease_id: LeaseId::new(1),
                return_to: BankReleaseDisposition::RomBank1,
            },
            &ctx,
            &mut state,
        ) {
            LoweringDisposition::Lowered(fragment) => fragment,
            other => panic!("unexpected lowering disposition: {other:?}"),
        };
        assert_eq!(fragment_bytes(&fragment).len(), 20);
    }

    #[test]
    fn lowering_assert_bank_label_only() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let fragment = match lowerer.lower_with_state(
            &PreLayoutOp::AssertBank {
                expected: MbcBankClass::Rom,
                expected_n: 3,
            },
            &ctx,
            &mut state,
        ) {
            LoweringDisposition::Lowered(fragment) => fragment,
            other => panic!("unexpected lowering disposition: {other:?}"),
        };
        assert!(fragment.instrs.is_empty());
        assert_eq!(fragment.labels.len(), 1);
    }

    #[test]
    fn lowering_assert_bank_emits_compare_and_trap_when_enabled() {
        let lowerer = BankingPreLayoutLowering::default()
            .with_assert_bank_policy(BankingAssertBankPolicy::CompareAndTrap);
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let fragment = match lowerer.lower_with_state(
            &PreLayoutOp::AssertBank {
                expected: MbcBankClass::Rom,
                expected_n: 0x0103,
            },
            &ctx,
            &mut state,
        ) {
            LoweringDisposition::Lowered(fragment) => fragment,
            other => panic!("unexpected lowering disposition: {other:?}"),
        };
        let bytes = fragment_bytes(&fragment);
        assert_eq!(
            bytes,
            vec![
                0xF0, 0x80, 0xFE, 0x03, 0x20, 0x06, 0xF0, 0x81, 0xFE, 0x01, 0x20, 0x00, 0xFF,
            ]
        );
        assert_eq!(fragment.labels.len(), 1);
    }

    #[test]
    fn lowering_assert_bank_compare_rejects_invalid_rom_bank() {
        let lowerer = BankingPreLayoutLowering::default()
            .with_assert_bank_policy(BankingAssertBankPolicy::CompareAndTrap);
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        assert!(matches!(
            lowerer.lower_with_state(
                &PreLayoutOp::AssertBank {
                    expected: MbcBankClass::Rom,
                    expected_n: 0,
                },
                &ctx,
                &mut state,
            ),
            LoweringDisposition::Error(LoweringError::Runtime(message))
                if message.contains("bank 0")
        ));
        assert!(matches!(
            lowerer.lower_with_state(
                &PreLayoutOp::AssertBank {
                    expected: MbcBankClass::Rom,
                    expected_n: 512,
                },
                &ctx,
                &mut state,
            ),
            LoweringDisposition::Error(LoweringError::Runtime(message))
                if message.contains("512")
        ));
    }

    #[test]
    fn lowering_rejects_non_banking_ops() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        assert_eq!(
            lowerer.lower_with_state(
                &PreLayoutOp::Yield {
                    kind: YieldKind::Cooperative
                },
                &ctx,
                &mut state,
            ),
            LoweringDisposition::NotOwned
        );
    }

    struct EmptyFallbackLowerer;

    impl DispositionPreLayoutOpLowering for EmptyFallbackLowerer {
        fn lower_disposition(
            &self,
            _op: &PreLayoutOp,
            _ctx: &LoweringContext<'_>,
        ) -> LoweringDisposition {
            LoweringDisposition::Lowered(LoweredFragment::default())
        }
    }

    #[test]
    fn composite_lowerer_does_not_swallow_error() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("banking", "normal_error").expect("section"),
        );
        let lease = LeaseId::new(1);
        builder.bank_lease(test_bank_lease_spec(lease, MbcBankClass::Rom, 3).expect("spec"));
        builder.bank_release_to(lease, BankReleaseDisposition::RomBank1);
        let banking = BankingPreLayoutLowering::default();
        let fallback = EmptyFallbackLowerer;
        let err = lower_pre_layout_ops_with_disposition(
            vec![builder.finish()],
            &[&banking, &fallback],
            &SymbolTable::new(),
        )
        .expect_err("banking error is terminal");
        assert!(
            matches!(err, LoweringError::Runtime(message) if message.contains("not Privileged"))
        );
    }

    #[test]
    fn lowering_revalidates_forged_bank_zero_wire_spec() {
        let lowerer = BankingPreLayoutLowering::default();
        let mut state = BankingLoweringState::default();
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let symbols = SymbolTable::new();
        let privilege = SectionPrivilege::privileged();
        let ctx = lowerer_ctx(
            SectionId::new(1),
            SectionRole::Bank0Nucleus,
            &privilege,
            &prov,
            &symbols,
        );
        let forged = test_bank_lease_spec(LeaseId::new(1), MbcBankClass::Rom, 0).expect("asm wire");
        assert!(matches!(
            lowerer.lower_with_state(&PreLayoutOp::BankLease(forged), &ctx, &mut state),
            LoweringDisposition::Error(LoweringError::Runtime(message))
                if message.contains("ROM bank 0")
        ));
    }

    #[test]
    fn lowering_rejects_forged_keep_current_release() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let lease = LeaseId::new(1);
        builder.bank_lease(test_bank_lease_spec(lease, MbcBankClass::Rom, 3).expect("spec"));
        builder.bank_release_to(lease, BankReleaseDisposition::KeepCurrent);
        let err = lower_pre_layout_ops(
            vec![builder.finish()],
            &BankingPreLayoutLowering::default(),
            &SymbolTable::new(),
        )
        .expect_err("keep-current requires trusted runtime proof");
        assert!(matches!(
            err,
            LoweringError::Runtime(message) if message.contains("keep-current")
        ));
    }

    #[test]
    fn full_acquire_release_round_trip() {
        let mut builder = banking_builder(SectionRole::Bank0Nucleus);
        let guard = lease_rom_switchable(
            &mut builder,
            ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("spec"),
        )
        .expect("lease");
        release_bank(&mut builder, guard, ReturnState::Rom(ReturnRomBank::Bank1)).expect("release");
        let section = builder.finish();
        let lowerer = BankingPreLayoutLowering::default();
        let lowered =
            lower_pre_layout_ops(vec![section], &lowerer, &SymbolTable::new()).expect("lowering");
        let bytes: Vec<u8> = lowered[0]
            .instrs
            .iter()
            .flat_map(|item| encode_instr(&item.data).expect("instr encodes"))
            .collect();
        assert_eq!(bytes.len(), 40);
        mbc_write_provenance_audit(&lowered, &InterruptSafetyTable::default(), &lowerer)
            .expect("audit");
    }

    #[test]
    fn mbc5_constants_match_gbf_hw() {
        assert_eq!(mbc5::MBC5_RAMG_BASE, 0x0000);
        assert_eq!(mbc5::MBC5_BANK1_BASE, 0x2000);
        assert_eq!(mbc5::MBC5_BANK2_BASE, 0x3000);
        assert_eq!(mbc5::MBC5_RAMB_BASE, 0x4000);
        assert_eq!(mbc5::MBC5_RAM_ENABLE_VALUE, 0x0A);
    }

    #[test]
    fn mbc_write_provenance_audit_catches_non_banking_source() {
        let section = LoweredSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Nucleus,
            name: SymbolName::runtime("banking", "bad").expect("name"),
            privilege: SectionPrivilege::privileged(),
            align: std::num::NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 1,
            labels: vec![],
            instrs: vec![OrderedItem::new(
                Instr::LdDirectFromA {
                    addr: direct(mbc5::MBC5_BANK1_BASE),
                },
                0,
                InstrProvenance::new(PlanningStage::Backend).with_source_op("not_banking"),
            )],
            data_blocks: vec![],
            alignments: vec![],
            legalization_ops: vec![],
            branches: vec![],
        };
        assert_eq!(
            mbc_write_provenance_audit(
                &[section],
                &InterruptSafetyTable::default(),
                &BankingPreLayoutLowering::default()
            ),
            Err(BankAbiViolation::MbcWriteOutsideBankingProvenance {
                section: SectionId::new(1)
            })
        );
    }

    #[test]
    fn mbc_write_provenance_audit_rejects_forged_public_source_string() {
        let section = LoweredSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Nucleus,
            name: SymbolName::runtime("banking", "forged").expect("name"),
            privilege: SectionPrivilege::privileged(),
            align: std::num::NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 1,
            labels: vec![],
            instrs: vec![OrderedItem::new(
                Instr::LdDirectFromA {
                    addr: direct(mbc5::MBC5_BANK1_BASE),
                },
                0,
                InstrProvenance::new(PlanningStage::Backend).with_source_op(BANKING_SOURCE_OP),
            )],
            data_blocks: vec![],
            alignments: vec![],
            legalization_ops: vec![],
            branches: vec![],
        };
        assert_eq!(
            mbc_write_provenance_audit(
                &[section],
                &InterruptSafetyTable::default(),
                &BankingPreLayoutLowering::default()
            ),
            Err(BankAbiViolation::MbcWriteOutsideBankingProvenance {
                section: SectionId::new(1)
            })
        );
    }

    #[test]
    fn mbc_write_provenance_audit_rejects_forged_public_source_and_note() {
        let section = LoweredSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Nucleus,
            name: SymbolName::runtime("banking", "forged_note").expect("name"),
            privilege: SectionPrivilege::privileged(),
            align: std::num::NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 1,
            labels: vec![],
            instrs: vec![OrderedItem::new(
                Instr::LdDirectFromA {
                    addr: direct(mbc5::MBC5_BANK1_BASE),
                },
                0,
                InstrProvenance::new(PlanningStage::Backend)
                    .with_source_op(BANKING_SOURCE_OP)
                    .with_note(alloc::format!("{BANKING_PROVENANCE_NOTE_PREFIX}:public")),
            )],
            data_blocks: vec![],
            alignments: vec![],
            legalization_ops: vec![],
            branches: vec![],
        };
        assert_eq!(
            mbc_write_provenance_audit(
                &[section],
                &InterruptSafetyTable::default(),
                &BankingPreLayoutLowering::default()
            ),
            Err(BankAbiViolation::MbcWriteOutsideBankingProvenance {
                section: SectionId::new(1)
            })
        );
    }
}
