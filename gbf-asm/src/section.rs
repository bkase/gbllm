//! Typed assembly sections and section items.

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::effect::{
    MachineEffect, MachineEffectKind, MbcRegisterClass, PrivilegeClass, classify_effect,
    classify_legalization_op, classify_pre_layout_op, privilege_of,
};
use crate::isa::{Cond, Instr};
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

    /// Whether this role permits `db` / `dw` inline data directives.
    ///
    /// Executable ROM sections (`Bank0Nucleus`, `CommonBank`, `ExpertBank`)
    /// reject inline data: an author could otherwise hand-encode a privileged
    /// instruction (e.g. `LD ($2000), A` as `db [0xEA, 0x00, 0x20]`) and slip
    /// it past the effect classifier — the same opaque-bytes escape hatch the
    /// removed `SectionItem::Raw` was meant to close. Inline ROM data tables
    /// (jump tables, fonts, lookup tables) live in their own non-executable
    /// sections instead. Closure-skill rule: "Raw bytes are opaque privileged
    /// effects unless a bead explicitly narrows the claim to data-only
    /// sections." This narrows the claim by section role.
    #[must_use]
    pub const fn permits_inline_data(self) -> bool {
        match self {
            Self::Bank0Nucleus | Self::CommonBank | Self::ExpertBank => false,
            Self::HeaderCartridge
            | Self::WramHotArena
            | Self::WramOverlay
            | Self::HramFastFlags
            | Self::SramPersistent
            | Self::VramOwnedByUi
            | Self::OamOwnedByUi => true,
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
    /// Writes to the MBC5 `$6000..=$7FFF` reserved window are forbidden in
    /// every section, including `Privileged`. The range is a hardware no-op on
    /// MBC5 and emitting a write there indicates either a bug or a leftover
    /// MBC1 assumption.
    ForbiddenMbcReserved,
}

/// Existing section item rejected by a replacement privilege policy.
///
/// `seq_index` is the global authoring sequence index of the offending item
/// (see `OrderedItem::seq_index`); it is stable across the SoA layout. When
/// multiple items violate the policy the earliest by `seq_index` is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SectionPrivilegeError {
    pub seq_index: u32,
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
        if matches!(
            effect,
            MachineEffect::StoreToMbcRegister {
                reg: MbcRegisterClass::Reserved
            }
        ) {
            return Err(PrivilegeViolation::ForbiddenMbcReserved);
        }

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

/// Bank class asserted or leased through runtime banking structured ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbcBankClass {
    Rom,
    Sram,
}

/// Builder-local bank lease id used to thread structured op ordering.
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

/// Trace probe identifier emitted by instrumentation structured ops.
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

/// Structured op markers lowered before layout.
///
/// These markers do not write MBC registers directly. They preserve authoring
/// intent until the BankLease ABI lowering owns concrete instruction emission.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreLayoutOp {
    BankLease(BankLeaseSpec),
    BankRelease {
        lease_id: LeaseId,
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
        expected_n: u16,
    },
}

/// Structured op markers lowered during legalization, after placement is known.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LegalizationOp {
    FarCall {
        target: SymbolName,
        lease_chain: Vec<LeaseId>,
    },
}

/// Authoring intent for a symbolic branch's emitted shape.
///
/// `Jump` items relax to `JR` when the resolved target is in range and to `JP`
/// otherwise. `Call` items relax to in-bank `CALL` when caller and callee land
/// in the same visible bank, and to a Bank0 far-call thunk otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchKind {
    Jump,
    Call,
}

/// Symbolic branch that keeps a target `SymbolName` until layout and relaxation
/// can pick the legal concrete encoding.
///
/// `Instr::JrRel`, `Instr::JpAbs`, and `Instr::Call` are concrete and require
/// resolved offsets/addresses, so they are not the right place to carry an
/// unresolved symbol. Branch relaxation consumes `SymbolicBranch` and emits
/// concrete `Instr`s (or a `LegalizationOp::FarCall` for cross-bank calls).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolicBranch {
    pub kind: BranchKind,
    pub cond: Option<Cond>,
    pub target: SymbolName,
}

impl SymbolicBranch {
    #[must_use]
    pub const fn new(kind: BranchKind, cond: Option<Cond>, target: SymbolName) -> Self {
        Self { kind, cond, target }
    }

    #[must_use]
    pub const fn machine_effect(&self) -> MachineEffect {
        match (self.kind, self.cond) {
            (BranchKind::Jump, None) => MachineEffect::UnconditionalBranch,
            (BranchKind::Jump, Some(_)) => MachineEffect::ConditionalBranch,
            (BranchKind::Call, _) => MachineEffect::Call,
        }
    }
}

/// Per-array item wrapper carrying a globally-monotone sequence index and
/// provenance.
///
/// The `Section` IR is laid out as a Struct-of-Arrays (SoA): each kind of item
/// (instructions, labels, data blocks, ...) lives in its own typed `Vec`. The
/// `seq_index` is unique within the owning `Section` and increases with every
/// emit, so callers can recover authoring order across the parallel arrays by
/// merging on `seq_index`. The SoA layout exists so stage-transition types
/// (`LoweredSection`, `LegalizedSection`) can *physically* drop arrays whose
/// items have been lowered away — the encoder consumes a `LegalizedSection`
/// that has no `pre_layout_ops` / `legalization_ops` / `branches` array at all,
/// making "encountered an un-legalized op" a compile-time impossibility instead
/// of a runtime error.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderedItem<T> {
    pub data: T,
    pub seq_index: u32,
    pub provenance: InstrProvenance,
}

impl<T> OrderedItem<T> {
    #[must_use]
    pub const fn new(data: T, seq_index: u32, provenance: InstrProvenance) -> Self {
        Self {
            data,
            seq_index,
            provenance,
        }
    }
}

/// Label marker attached to the next emitted byte position.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Label {
    pub id: SymbolId,
    pub name: SymbolName,
}

/// Inline data directive. `Bytes` is `db` (8-bit), `Words` is `dw` (16-bit
/// little-endian on emit). Merged into one array so the layout/encoder can
/// treat all data-only items uniformly without juggling two parallel arrays.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum DataBlock {
    Bytes(Vec<u8>),
    Words(Vec<u16>),
}

impl DataBlock {
    /// Number of ROM bytes this data block contributes pre-layout. Always known.
    #[must_use]
    pub fn byte_len(&self) -> u32 {
        match self {
            Self::Bytes(b) => b.len() as u32,
            Self::Words(w) => (w.len() as u32) * 2,
        }
    }
}

/// Section-local alignment directive. The actual padding count is decided by
/// layout once the cursor is known; this carries only the requested alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Align(pub NonZeroU16);

/// Borrowed view of an item in `seq_index` order.
///
/// `Section::iter_items()` returns `SectionItemView`s by k-way-merging the
/// parallel arrays on `seq_index`. The view exists for code that genuinely
/// needs ordered traversal (listing, JSON-encoded debug dumps, the legacy test
/// surface). Stage-transformations and the encoder iterate the typed arrays
/// directly — they do not pay the merge cost.
#[derive(Debug, Clone, Copy)]
pub enum SectionItemView<'a> {
    Label(&'a OrderedItem<Label>),
    Instr(&'a OrderedItem<Instr>),
    DataBlock(&'a OrderedItem<DataBlock>),
    Align(&'a OrderedItem<Align>),
    PreLayoutOp(&'a OrderedItem<PreLayoutOp>),
    LegalizationOp(&'a OrderedItem<LegalizationOp>),
    Branch(&'a OrderedItem<SymbolicBranch>),
}

impl<'a> SectionItemView<'a> {
    #[must_use]
    pub const fn seq_index(&self) -> u32 {
        match self {
            Self::Label(o) => o.seq_index,
            Self::Instr(o) => o.seq_index,
            Self::DataBlock(o) => o.seq_index,
            Self::Align(o) => o.seq_index,
            Self::PreLayoutOp(o) => o.seq_index,
            Self::LegalizationOp(o) => o.seq_index,
            Self::Branch(o) => o.seq_index,
        }
    }

    #[must_use]
    pub const fn provenance(&self) -> &'a InstrProvenance {
        match self {
            Self::Label(o) => &o.provenance,
            Self::Instr(o) => &o.provenance,
            Self::DataBlock(o) => &o.provenance,
            Self::Align(o) => &o.provenance,
            Self::PreLayoutOp(o) => &o.provenance,
            Self::LegalizationOp(o) => &o.provenance,
            Self::Branch(o) => &o.provenance,
        }
    }

    #[must_use]
    pub fn machine_effect(&self) -> Option<MachineEffect> {
        match self {
            Self::Instr(o) => Some(classify_effect(&o.data)),
            Self::PreLayoutOp(o) => Some(classify_pre_layout_op(&o.data)),
            Self::LegalizationOp(o) => Some(classify_legalization_op(&o.data)),
            Self::Branch(o) => Some(o.data.machine_effect()),
            Self::Label(_) | Self::DataBlock(_) | Self::Align(_) => None,
        }
    }

    #[must_use]
    pub fn fixed_byte_len(&self) -> Option<u32> {
        match self {
            Self::Label(_) => Some(0),
            Self::Instr(o) => Some(u32::from(o.data.byte_len())),
            Self::DataBlock(o) => Some(o.data.byte_len()),
            Self::Align(_) | Self::PreLayoutOp(_) | Self::LegalizationOp(_) | Self::Branch(_) => {
                None
            }
        }
    }
}

/// Typed section with a validated alignment, laid out as a Struct of Arrays.
///
/// Items are partitioned by kind into parallel `Vec<OrderedItem<T>>` arrays.
/// Authoring order is recoverable via each `OrderedItem`'s `seq_index`. The
/// SoA layout exists so stage-transition types (`LoweredSection`,
/// `LegalizedSection`) can drop entire arrays — for example,
/// `LegalizedSection` *has no* `pre_layout_ops`, `legalization_ops`, or
/// `branches` field, so the encoder cannot encounter an un-legalized op even
/// if it tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    id: SectionId,
    role: SectionRole,
    name: SymbolName,
    #[serde(default)]
    privilege: SectionPrivilege,
    align: NonZeroU16,
    size_hint_bytes: Option<u32>,
    next_seq_index: u32,
    labels: Vec<OrderedItem<Label>>,
    instrs: Vec<OrderedItem<Instr>>,
    data_blocks: Vec<OrderedItem<DataBlock>>,
    alignments: Vec<OrderedItem<Align>>,
    pre_layout_ops: Vec<OrderedItem<PreLayoutOp>>,
    legalization_ops: Vec<OrderedItem<LegalizationOp>>,
    branches: Vec<OrderedItem<SymbolicBranch>>,
}

impl Section {
    pub fn new(id: SectionId, role: SectionRole, name: SymbolName, align: NonZeroU16) -> Self {
        Self {
            id,
            role,
            name,
            privilege: SectionPrivilege::default(),
            align,
            size_hint_bytes: None,
            next_seq_index: 0,
            labels: Vec::new(),
            instrs: Vec::new(),
            data_blocks: Vec::new(),
            alignments: Vec::new(),
            pre_layout_ops: Vec::new(),
            legalization_ops: Vec::new(),
            branches: Vec::new(),
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
        validate_section_for_privilege(self, &privilege)?;
        self.privilege = privilege;
        Ok(())
    }

    fn next_seq(&mut self) -> u32 {
        let idx = self.next_seq_index;
        self.next_seq_index = self
            .next_seq_index
            .checked_add(1)
            .expect("section seq_index overflowed u32");
        idx
    }

    pub(crate) fn push_label(&mut self, label: Label, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.labels.push(OrderedItem::new(label, seq, provenance));
    }

    pub(crate) fn push_instr(&mut self, instr: Instr, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.instrs.push(OrderedItem::new(instr, seq, provenance));
    }

    pub(crate) fn push_data_block(&mut self, block: DataBlock, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.data_blocks
            .push(OrderedItem::new(block, seq, provenance));
    }

    pub(crate) fn push_align(&mut self, align: Align, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.alignments
            .push(OrderedItem::new(align, seq, provenance));
    }

    pub(crate) fn push_pre_layout_op(&mut self, op: PreLayoutOp, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.pre_layout_ops
            .push(OrderedItem::new(op, seq, provenance));
    }

    pub(crate) fn push_legalization_op(&mut self, op: LegalizationOp, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.legalization_ops
            .push(OrderedItem::new(op, seq, provenance));
    }

    pub(crate) fn push_branch(&mut self, branch: SymbolicBranch, provenance: InstrProvenance) {
        let seq = self.next_seq();
        self.branches
            .push(OrderedItem::new(branch, seq, provenance));
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
    pub fn labels(&self) -> &[OrderedItem<Label>] {
        &self.labels
    }

    #[must_use]
    pub fn instrs(&self) -> &[OrderedItem<Instr>] {
        &self.instrs
    }

    #[must_use]
    pub fn data_blocks(&self) -> &[OrderedItem<DataBlock>] {
        &self.data_blocks
    }

    #[must_use]
    pub fn alignments(&self) -> &[OrderedItem<Align>] {
        &self.alignments
    }

    #[must_use]
    pub fn pre_layout_ops(&self) -> &[OrderedItem<PreLayoutOp>] {
        &self.pre_layout_ops
    }

    #[must_use]
    pub fn legalization_ops(&self) -> &[OrderedItem<LegalizationOp>] {
        &self.legalization_ops
    }

    #[must_use]
    pub fn branches(&self) -> &[OrderedItem<SymbolicBranch>] {
        &self.branches
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
    pub fn total_items(&self) -> usize {
        self.labels.len()
            + self.instrs.len()
            + self.data_blocks.len()
            + self.alignments.len()
            + self.pre_layout_ops.len()
            + self.legalization_ops.len()
            + self.branches.len()
    }

    /// Returns the per-array borrowed items merged in `seq_index` order.
    ///
    /// O(N log K) for N items across K arrays — used by listing and tests
    /// that genuinely need authoring order. The encoder and stage
    /// transformations operate on the typed arrays directly.
    #[must_use]
    pub fn iter_items(&self) -> Vec<SectionItemView<'_>> {
        let mut views: Vec<SectionItemView<'_>> = Vec::with_capacity(self.total_items());
        views.extend(self.labels.iter().map(SectionItemView::Label));
        views.extend(self.instrs.iter().map(SectionItemView::Instr));
        views.extend(self.data_blocks.iter().map(SectionItemView::DataBlock));
        views.extend(self.alignments.iter().map(SectionItemView::Align));
        views.extend(self.pre_layout_ops.iter().map(SectionItemView::PreLayoutOp));
        views.extend(
            self.legalization_ops
                .iter()
                .map(SectionItemView::LegalizationOp),
        );
        views.extend(self.branches.iter().map(SectionItemView::Branch));
        views.sort_by_key(SectionItemView::seq_index);
        views
    }

    /// Returns `Some(N)` only when every item has a known fixed pre-layout
    /// width — i.e., the section contains no `Align`, `PreLayoutOp`,
    /// `LegalizationOp`, or `Branch` items. Otherwise returns `None`.
    #[must_use]
    pub fn fixed_item_bytes(&self) -> Option<u32> {
        if !self.alignments.is_empty()
            || !self.pre_layout_ops.is_empty()
            || !self.legalization_ops.is_empty()
            || !self.branches.is_empty()
        {
            return None;
        }
        let instr_bytes: u32 = self
            .instrs
            .iter()
            .map(|item| u32::from(item.data.byte_len()))
            .sum();
        let data_bytes: u32 = self
            .data_blocks
            .iter()
            .map(|item| item.data.byte_len())
            .sum();
        Some(instr_bytes + data_bytes)
    }
}

/// Section after pre-layout lowering. The `pre_layout_ops` array is physically
/// absent: every `PreLayoutOp` has been replaced with `Instr`s, `DataBlock`s,
/// or `LegalizationOp`s. Layout and relax consume `LoweredSection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoweredSection {
    pub id: SectionId,
    pub role: SectionRole,
    pub name: SymbolName,
    pub align: NonZeroU16,
    pub size_hint_bytes: Option<u32>,
    pub next_seq_index: u32,
    pub labels: Vec<OrderedItem<Label>>,
    pub instrs: Vec<OrderedItem<Instr>>,
    pub data_blocks: Vec<OrderedItem<DataBlock>>,
    pub alignments: Vec<OrderedItem<Align>>,
    pub legalization_ops: Vec<OrderedItem<LegalizationOp>>,
    pub branches: Vec<OrderedItem<SymbolicBranch>>,
}

/// Encoder-ready section. The `pre_layout_ops`, `legalization_ops`, and
/// `branches` arrays are *physically absent* — the encoder pattern-matches
/// only over `labels`, `instrs`, `data_blocks`, and `alignments`, and the
/// match is exhaustive at compile time. There is no `EncodeError::OpNotLegalized`
/// because the un-legalized variants do not appear in the type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalizedSection {
    pub id: SectionId,
    pub role: SectionRole,
    pub name: SymbolName,
    pub align: NonZeroU16,
    pub size_hint_bytes: Option<u32>,
    pub next_seq_index: u32,
    pub labels: Vec<OrderedItem<Label>>,
    pub instrs: Vec<OrderedItem<Instr>>,
    pub data_blocks: Vec<OrderedItem<DataBlock>>,
    pub alignments: Vec<OrderedItem<Align>>,
}

fn validate_section_for_privilege(
    section: &Section,
    privilege: &SectionPrivilege,
) -> Result<(), SectionPrivilegeError> {
    let candidates = section
        .instrs
        .iter()
        .map(|i| (i.seq_index, classify_effect(&i.data)))
        .chain(
            section
                .pre_layout_ops
                .iter()
                .map(|i| (i.seq_index, classify_pre_layout_op(&i.data))),
        )
        .chain(
            section
                .legalization_ops
                .iter()
                .map(|i| (i.seq_index, classify_legalization_op(&i.data))),
        )
        .chain(
            section
                .branches
                .iter()
                .map(|i| (i.seq_index, i.data.machine_effect())),
        );

    let mut earliest: Option<(u32, MachineEffect, PrivilegeViolation)> = None;
    for (seq_index, effect) in candidates {
        if let Err(violation) = privilege.check_effect(effect) {
            match earliest {
                Some((existing_seq, _, _)) if existing_seq <= seq_index => {}
                _ => earliest = Some((seq_index, effect, violation)),
            }
        }
    }

    if let Some((seq_index, effect, violation)) = earliest {
        Err(SectionPrivilegeError {
            seq_index,
            effect,
            violation,
        })
    } else {
        Ok(())
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
fn reserved_mbc_writes_are_forbidden_even_in_privileged_sections() {
    let reserved_write = MachineEffect::StoreToMbcRegister {
        reg: MbcRegisterClass::Reserved,
    };

    for privilege in [
        SectionPrivilege::normal(),
        SectionPrivilege::privileged(),
        SectionPrivilege::interrupt_handler(),
    ] {
        assert_eq!(
            privilege.check_effect(reserved_write),
            Err(PrivilegeViolation::ForbiddenMbcReserved),
            "{privilege:?} must reject reserved MBC writes",
        );
    }
}

#[cfg(test)]
#[test]
fn symbolic_branch_classifies_and_round_trips() {
    use crate::provenance::{InstrProvenance, PlanningStage};

    let provenance = InstrProvenance::new(PlanningStage::Backend).with_source_op("emit_branch");
    let target = SymbolName::runtime("loop", "head").expect("target name");

    let cases = [
        (
            SymbolicBranch::new(BranchKind::Jump, None, target.clone()),
            MachineEffect::UnconditionalBranch,
        ),
        (
            SymbolicBranch::new(BranchKind::Jump, Some(Cond::NZ), target.clone()),
            MachineEffect::ConditionalBranch,
        ),
        (
            SymbolicBranch::new(BranchKind::Call, None, target.clone()),
            MachineEffect::Call,
        ),
        (
            SymbolicBranch::new(BranchKind::Call, Some(Cond::C), target.clone()),
            MachineEffect::Call,
        ),
    ];

    for (branch, expected_effect) in cases {
        let item = OrderedItem::new(branch.clone(), 0, provenance.clone());
        assert_eq!(item.data.machine_effect(), expected_effect);

        let view = SectionItemView::Branch(&item);
        assert_eq!(view.machine_effect(), Some(expected_effect));
        assert_eq!(view.fixed_byte_len(), None);
        assert_eq!(view.provenance(), &provenance);

        let encoded = serde_json::to_string(&item).expect("branch ordered item serializes");
        let decoded: OrderedItem<SymbolicBranch> =
            serde_json::from_str(&encoded).expect("branch ordered item deserializes");
        assert_eq!(decoded, item);
    }
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

    section.push_instr(Instr::Nop, provenance.clone());
    section.push_data_block(DataBlock::Bytes(vec![0xCE, 0xED]), provenance.clone());
    section.push_data_block(DataBlock::Words(vec![0x1234, 0xCAFE]), provenance.clone());

    assert_eq!(section.fixed_item_bytes(), Some(7));

    let ordered = section.iter_items();
    assert_eq!(ordered.len(), 3);
    assert_eq!(ordered[0].provenance(), &provenance);
    assert_eq!(ordered[0].seq_index(), 0);
    assert_eq!(ordered[2].seq_index(), 2);

    section.push_pre_layout_op(
        PreLayoutOp::Yield {
            kind: YieldKind::Cooperative,
        },
        provenance.clone(),
    );
    assert_eq!(section.fixed_item_bytes(), None);
    let ordered = section.iter_items();
    assert_eq!(ordered.len(), 4);
    assert_eq!(
        ordered[3].machine_effect(),
        Some(MachineEffect::SystemCall(SystemCallKind::Yield))
    );

    let encoded = serde_json::to_string(&section).expect("section serializes");
    let decoded: Section = serde_json::from_str(&encoded).expect("section deserializes");
    assert_eq!(decoded, section);
}

#[cfg(test)]
#[test]
fn legalized_section_drops_unencoded_arrays_at_the_type_level() {
    // The structural guarantee: LegalizedSection has no fields for
    // pre_layout_ops, legalization_ops, or branches. Attempting to construct
    // one with such an array is a compile error. This test pins the surface
    // so a future refactor cannot quietly bring the un-legalized arrays back.
    use crate::isa::Instr;
    use crate::provenance::{InstrProvenance, PlanningStage};

    let provenance = InstrProvenance::new(PlanningStage::Backend).with_source_op("legalized");
    let legalized = LegalizedSection {
        id: SectionId::new(1),
        role: SectionRole::Bank0Nucleus,
        name: SymbolName::runtime("boot", "entry").expect("name"),
        align: NonZeroU16::new(1).expect("align"),
        size_hint_bytes: None,
        next_seq_index: 1,
        labels: vec![],
        instrs: vec![OrderedItem::new(Instr::Nop, 0, provenance)],
        data_blocks: vec![],
        alignments: vec![],
    };
    assert_eq!(legalized.instrs.len(), 1);

    let json = serde_json::to_string(&legalized).expect("legalized serializes");
    assert!(
        !json.contains("pre_layout_ops"),
        "LegalizedSection JSON must not contain pre_layout_ops"
    );
    assert!(
        !json.contains("legalization_ops"),
        "LegalizedSection JSON must not contain legalization_ops"
    );
    assert!(
        !json.contains("branches"),
        "LegalizedSection JSON must not contain branches"
    );
}
