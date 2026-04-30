//! Ergonomic typed builder for assembly sections.
//!
//! The builder is the authoring surface for runtime authors and compiler
//! lowering code. It preserves typed `Instr`/structured op values and attaches
//! provenance to every emitted item.
//!
//! Output is symbolic pre-layout section IR. This layer records label markers,
//! alignment directives, and structured op intent. It performs local section
//! privilege/effect validation for typed instructions and structured ops; it does
//! not perform relocation, branch relaxation, far-call thunk insertion,
//! dynamic-address reachability proofs, or final byte lowering. There is no
//! opaque-byte escape hatch — every emitted byte goes through `Instr`, `db`, or
//! `dw`.

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU16;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use crate::effect::{
    MachineEffect, classify_effect, classify_legalization_op, classify_pre_layout_op,
};
use crate::isa::Instr;
use crate::provenance::{InstrProvenance, PlanningStage};
use crate::section::{
    Align, BankLeaseSpec, DataBlock, Label, LeaseId, LegalizationOp, MbcBankClass, PreLayoutOp,
    PrivilegeViolation, ProbeLevel, Section, SectionId, SectionPrivilege, SectionPrivilegeError,
    SectionRole, SymbolId, SymbolicBranch, TraceProbeId, YieldKind,
};
use crate::symbols::SymbolName;

/// Builder construction or emission error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderError {
    ZeroAlignment,
    DuplicateLabel {
        name: SymbolName,
    },
    TooManyLabels,
    DuplicateLease {
        lease_id: LeaseId,
    },
    UnknownLease {
        lease_id: LeaseId,
    },
    SramBankOutOfRange {
        bank: u16,
    },
    RomBankOutOfRange {
        bank: u16,
    },
    PrivilegeViolation {
        effect: MachineEffect,
        violation: PrivilegeViolation,
    },
    SectionPrivilegeViolation(SectionPrivilegeError),
    /// `db` / `dw` was called inside a section whose `SectionRole` is
    /// executable (Bank0Nucleus, CommonBank, ExpertBank). Inline data in an
    /// executable section is the opaque-bytes escape hatch we closed by
    /// removing `SectionItem::Raw` — emit data tables in their own
    /// non-executable sections instead.
    InlineDataInExecutableSection {
        role: SectionRole,
    },
}

impl fmt::Display for BuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroAlignment => f.write_str("alignment must be nonzero"),
            Self::DuplicateLabel { name } => write!(f, "label {name} already exists"),
            Self::TooManyLabels => f.write_str("too many labels emitted by one builder"),
            Self::DuplicateLease { lease_id } => {
                write!(f, "lease id {} is already active", lease_id.get())
            }
            Self::UnknownLease { lease_id } => {
                write!(f, "lease id {} is not active", lease_id.get())
            }
            Self::SramBankOutOfRange { bank } => {
                write!(f, "SRAM bank {bank} is outside MBC5 range 0..=15")
            }
            Self::RomBankOutOfRange { bank } => {
                write!(f, "ROM bank {bank} is outside MBC5 range 0..=511")
            }
            Self::PrivilegeViolation { effect, violation } => {
                write!(
                    f,
                    "section privilege rejected effect {effect:?}: {violation:?}"
                )
            }
            Self::SectionPrivilegeViolation(error) => {
                write!(
                    f,
                    "section privilege rejected existing item at seq {} with effect {:?}: {:?}",
                    error.seq_index, error.effect, error.violation
                )
            }
            Self::InlineDataInExecutableSection { role } => {
                write!(
                    f,
                    "inline db/dw not allowed in executable section role {role:?}; \
                     emit data into a non-executable section instead"
                )
            }
        }
    }
}

impl std::error::Error for BuilderError {}

/// Typed section builder.
#[derive(Debug, Clone)]
pub struct Builder {
    section: Section,
    cur_provenance: InstrProvenance,
    next_label_id: u32,
    labels: BTreeSet<SymbolName>,
    active_leases: BTreeSet<LeaseId>,
}

impl Builder {
    /// Creates a section with id `0` and section alignment `1`.
    pub fn new(role: SectionRole, name: SymbolName) -> Self {
        Self::new_with_id(SectionId::new(0), role, name)
    }

    pub fn new_with_id(id: SectionId, role: SectionRole, name: SymbolName) -> Self {
        Self {
            section: Section::new(id, role, name, NonZeroU16::new(1).expect("1 is nonzero")),
            cur_provenance: InstrProvenance::new(PlanningStage::Backend)
                .with_source_op("builder.default"),
            next_label_id: 0,
            labels: BTreeSet::new(),
            active_leases: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn current_provenance(&self) -> &InstrProvenance {
        &self.cur_provenance
    }

    #[must_use]
    pub fn with_section_privilege(mut self, privilege: SectionPrivilege) -> Self {
        self.try_set_section_privilege(privilege)
            .expect("section privilege rejected an existing item");
        self
    }

    pub fn set_section_privilege(&mut self, privilege: SectionPrivilege) {
        self.try_set_section_privilege(privilege)
            .expect("section privilege rejected an existing item");
    }

    pub fn try_set_section_privilege(
        &mut self,
        privilege: SectionPrivilege,
    ) -> Result<(), BuilderError> {
        self.section
            .set_privilege(privilege)
            .map_err(BuilderError::SectionPrivilegeViolation)
    }

    pub fn emit(&mut self, instr: Instr) {
        self.try_emit(instr)
            .expect("section privilege rejected instruction in Builder::emit");
    }

    pub fn try_emit(&mut self, instr: Instr) -> Result<(), BuilderError> {
        self.validate_effect(classify_effect(&instr))?;
        self.section.push_instr(instr, self.cur_provenance.clone());
        Ok(())
    }

    pub fn db(&mut self, byte: u8) {
        self.try_db(byte)
            .expect("db rejected by section role; use try_db");
    }

    pub fn try_db(&mut self, byte: u8) -> Result<(), BuilderError> {
        self.try_db_bytes([byte])
    }

    pub fn db_bytes(&mut self, bytes: impl Into<Vec<u8>>) {
        self.try_db_bytes(bytes)
            .expect("db_bytes rejected by section role; use try_db_bytes");
    }

    pub fn try_db_bytes(&mut self, bytes: impl Into<Vec<u8>>) -> Result<(), BuilderError> {
        self.guard_inline_data()?;
        self.section
            .push_data_block(DataBlock::Bytes(bytes.into()), self.cur_provenance.clone());
        Ok(())
    }

    pub fn dw(&mut self, word: u16) {
        self.try_dw(word)
            .expect("dw rejected by section role; use try_dw");
    }

    pub fn try_dw(&mut self, word: u16) -> Result<(), BuilderError> {
        self.try_dw_words([word])
    }

    pub fn dw_words(&mut self, words: impl Into<Vec<u16>>) {
        self.try_dw_words(words)
            .expect("dw_words rejected by section role; use try_dw_words");
    }

    pub fn try_dw_words(&mut self, words: impl Into<Vec<u16>>) -> Result<(), BuilderError> {
        self.guard_inline_data()?;
        self.section
            .push_data_block(DataBlock::Words(words.into()), self.cur_provenance.clone());
        Ok(())
    }

    fn guard_inline_data(&self) -> Result<(), BuilderError> {
        let role = self.section.role();
        if !role.permits_inline_data() {
            return Err(BuilderError::InlineDataInExecutableSection { role });
        }
        Ok(())
    }

    /// Emits a label marker and returns its builder-local id.
    ///
    /// Panics if the same label name is emitted twice. Use `try_label` when the
    /// name comes from untrusted input.
    pub fn label(&mut self, name: SymbolName) -> SymbolId {
        self.try_label(name)
            .expect("duplicate label emitted through Builder::label")
    }

    pub fn try_label(&mut self, name: SymbolName) -> Result<SymbolId, BuilderError> {
        if !self.labels.insert(name.clone()) {
            return Err(BuilderError::DuplicateLabel { name });
        }

        let id = SymbolId::new(self.next_label_id);
        self.next_label_id = self
            .next_label_id
            .checked_add(1)
            .ok_or(BuilderError::TooManyLabels)?;
        self.section
            .push_label(Label { id, name }, self.cur_provenance.clone());
        Ok(id)
    }

    pub fn align(&mut self, align: NonZeroU16) {
        self.section
            .push_align(Align(align), self.cur_provenance.clone());
    }

    pub fn try_align(&mut self, align: u16) -> Result<(), BuilderError> {
        let align = NonZeroU16::new(align).ok_or(BuilderError::ZeroAlignment)?;
        self.align(align);
        Ok(())
    }

    pub fn with_provenance<R>(
        &mut self,
        provenance: InstrProvenance,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = std::mem::replace(&mut self.cur_provenance, provenance);
        let result = catch_unwind(AssertUnwindSafe(|| f(self)));
        self.cur_provenance = previous;

        match result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        }
    }

    pub fn bank_lease(&mut self, lease: BankLeaseSpec) {
        self.try_bank_lease(lease)
            .expect("invalid lease lifecycle in Builder::bank_lease");
    }

    pub fn try_bank_lease(&mut self, lease: BankLeaseSpec) -> Result<(), BuilderError> {
        let op = PreLayoutOp::BankLease(lease.clone());
        self.validate_effect(classify_pre_layout_op(&op))?;
        let lease_id = lease.lease_id();
        if !self.active_leases.insert(lease_id) {
            return Err(BuilderError::DuplicateLease { lease_id });
        }

        self.pre_layout_op_unchecked(op);
        Ok(())
    }

    pub fn bank_release(&mut self, lease_id: LeaseId) {
        self.try_bank_release(lease_id)
            .expect("invalid lease lifecycle in Builder::bank_release");
    }

    pub fn try_bank_release(&mut self, lease_id: LeaseId) -> Result<(), BuilderError> {
        let op = PreLayoutOp::BankRelease { lease_id };
        self.validate_effect(classify_pre_layout_op(&op))?;
        if !self.active_leases.remove(&lease_id) {
            return Err(BuilderError::UnknownLease { lease_id });
        }

        self.pre_layout_op_unchecked(op);
        Ok(())
    }

    /// Emits a symbolic branch that keeps `target` unresolved until relaxation.
    ///
    /// Branch relaxation chooses the concrete encoding (`JR`/`JP`/in-bank
    /// `CALL`/far-call thunk) once layout assigns addresses and banks.
    pub fn branch(&mut self, branch: SymbolicBranch) {
        self.try_branch(branch)
            .expect("section privilege rejected symbolic branch");
    }

    pub fn try_branch(&mut self, branch: SymbolicBranch) -> Result<(), BuilderError> {
        self.validate_effect(branch.machine_effect())?;
        self.section
            .push_branch(branch, self.cur_provenance.clone());
        Ok(())
    }

    pub fn far_call(&mut self, target: SymbolName, lease_chain: &[LeaseId]) {
        self.try_far_call(target, lease_chain)
            .expect("invalid lease chain in Builder::far_call");
    }

    pub fn try_far_call(
        &mut self,
        target: SymbolName,
        lease_chain: &[LeaseId],
    ) -> Result<(), BuilderError> {
        let op = LegalizationOp::FarCall {
            target,
            lease_chain: lease_chain.to_vec(),
        };
        self.validate_effect(classify_legalization_op(&op))?;
        for lease_id in lease_chain {
            if !self.active_leases.contains(lease_id) {
                return Err(BuilderError::UnknownLease {
                    lease_id: *lease_id,
                });
            }
        }

        self.legalization_op_unchecked(op);
        Ok(())
    }

    pub fn yield_op(&mut self, kind: YieldKind) {
        self.try_yield_op(kind)
            .expect("section privilege rejected yield structured op");
    }

    pub fn try_yield_op(&mut self, kind: YieldKind) -> Result<(), BuilderError> {
        self.pre_layout_op(PreLayoutOp::Yield { kind })
    }

    pub fn trace_probe(&mut self, id: TraceProbeId, level: ProbeLevel) {
        self.try_trace_probe(id, level)
            .expect("section privilege rejected trace-probe structured op");
    }

    pub fn try_trace_probe(
        &mut self,
        id: TraceProbeId,
        level: ProbeLevel,
    ) -> Result<(), BuilderError> {
        self.pre_layout_op(PreLayoutOp::TraceProbe { id, level })
    }

    pub fn assert_bank(&mut self, expected: MbcBankClass, expected_n: u16) {
        self.try_assert_bank(expected, expected_n)
            .expect("invalid bank assertion in Builder::assert_bank");
    }

    pub fn try_assert_bank(
        &mut self,
        expected: MbcBankClass,
        expected_n: u16,
    ) -> Result<(), BuilderError> {
        let op = PreLayoutOp::AssertBank {
            expected,
            expected_n,
        };
        self.validate_effect(classify_pre_layout_op(&op))?;
        if expected == MbcBankClass::Rom && expected_n > BankLeaseSpec::MAX_ROM_BANK {
            return Err(BuilderError::RomBankOutOfRange { bank: expected_n });
        }
        if expected == MbcBankClass::Sram && expected_n > BankLeaseSpec::MAX_SRAM_BANK {
            return Err(BuilderError::SramBankOutOfRange { bank: expected_n });
        }

        self.pre_layout_op_unchecked(op);
        Ok(())
    }

    pub fn finish(self) -> Section {
        self.section
    }

    fn validate_effect(&self, effect: MachineEffect) -> Result<(), BuilderError> {
        self.section
            .privilege()
            .check_effect(effect)
            .map_err(|violation| BuilderError::PrivilegeViolation { effect, violation })
    }

    fn pre_layout_op(&mut self, op: PreLayoutOp) -> Result<(), BuilderError> {
        self.validate_effect(classify_pre_layout_op(&op))?;
        self.pre_layout_op_unchecked(op);
        Ok(())
    }

    fn pre_layout_op_unchecked(&mut self, op: PreLayoutOp) {
        self.section
            .push_pre_layout_op(op, self.cur_provenance.clone());
    }

    fn legalization_op_unchecked(&mut self, op: LegalizationOp) {
        self.section
            .push_legalization_op(op, self.cur_provenance.clone());
    }
}

#[cfg(test)]
#[test]
fn roundtrip() {
    use crate::isa::Reg8;
    use crate::section::DataBlock;

    // HeaderCartridge is a data-only role that accepts both Instr (the
    // entry-point thunk at $0100 is `nop; jp $0150`, which is legitimately
    // expressed as Instrs) and inline `db`/`dw` for the cartridge header
    // payload. Executable roles (Bank0Nucleus/CommonBank/ExpertBank) reject
    // inline data — see `try_db_in_executable_section_is_rejected` below.
    let mut builder = Builder::new(
        SectionRole::HeaderCartridge,
        SymbolName::runtime("boot", "start").expect("section name"),
    );
    let entry = builder.label(SymbolName::runtime("boot", "entry").expect("label"));
    builder.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0x12,
    });
    builder.db(0x34);
    builder.dw(0x5678);
    builder.try_align(4).expect("valid alignment");

    let section = builder.finish();
    let default_provenance =
        InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default");

    assert_eq!(section.role(), SectionRole::HeaderCartridge);
    assert_eq!(section.total_items(), 5);
    assert_eq!(section.fixed_item_bytes(), None);
    assert_eq!(entry.get(), 0);

    assert_eq!(section.labels().len(), 1);
    let label = &section.labels()[0];
    assert_eq!(label.seq_index, 0);
    assert_eq!(label.data.id, entry);
    assert_eq!(
        label.data.name,
        SymbolName::runtime("boot", "entry").expect("label")
    );
    assert_eq!(label.provenance, default_provenance);

    assert_eq!(section.instrs().len(), 1);
    let instr = &section.instrs()[0];
    assert_eq!(instr.seq_index, 1);
    assert_eq!(
        instr.data,
        Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: 0x12
        }
    );

    assert_eq!(section.data_blocks().len(), 2);
    assert_eq!(section.data_blocks()[0].seq_index, 2);
    assert_eq!(section.data_blocks()[0].data, DataBlock::Bytes(vec![0x34]));
    assert_eq!(section.data_blocks()[1].seq_index, 3);
    assert_eq!(
        section.data_blocks()[1].data,
        DataBlock::Words(vec![0x5678])
    );

    assert_eq!(section.alignments().len(), 1);
    assert_eq!(section.alignments()[0].seq_index, 4);
}

#[cfg(test)]
#[test]
fn provenance_recorded() {
    let default_provenance =
        InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default");
    let storage_provenance =
        InstrProvenance::new(PlanningStage::StoragePlan).with_source_op("arena_bind");

    // The provenance-scope semantics are independent of role; pick a data-only
    // role so `db`/`dw` are accepted (Option A: executable sections reject
    // inline data — see SectionRole::permits_inline_data).
    let mut builder = Builder::new(
        SectionRole::HeaderCartridge,
        SymbolName::section(SectionRole::HeaderCartridge, SectionId::new(0)).expect("section name"),
    );
    builder.emit(Instr::Nop);
    builder.with_provenance(storage_provenance.clone(), |builder| {
        builder.db(0xAA);
        builder.db_bytes([0xBB, 0xCC]);
    });
    builder.dw(0x1234);

    let section = builder.finish();
    let ordered = section.iter_items();

    assert_eq!(ordered.len(), 4);
    assert_eq!(ordered[0].provenance(), &default_provenance);
    assert_eq!(ordered[1].provenance(), &storage_provenance);
    assert_eq!(ordered[2].provenance(), &storage_provenance);
    assert_eq!(ordered[3].provenance(), &default_provenance);
    assert!(
        ordered
            .iter()
            .all(|item| item.provenance().source_op.is_some())
    );
}

#[cfg(test)]
#[test]
fn structured_ops_record_exact_payloads() {
    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::runtime("banking", "ops").expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    let lease = LeaseId::new(7);
    let target = SymbolName::runtime("expert", "enter").expect("target");
    builder.bank_lease(BankLeaseSpec::new(lease, MbcBankClass::Rom, 12).expect("lease"));
    builder.assert_bank(MbcBankClass::Rom, 12);
    builder.trace_probe(TraceProbeId::new(3), ProbeLevel::Debug);
    builder.yield_op(YieldKind::PollInterrupts);
    builder.far_call(target.clone(), &[lease]);
    builder.bank_release(lease);

    let section = builder.finish();

    assert_eq!(section.total_items(), 6);
    assert_eq!(section.pre_layout_ops().len(), 5);
    assert_eq!(section.legalization_ops().len(), 1);

    let pre_payloads: Vec<&PreLayoutOp> = section
        .pre_layout_ops()
        .iter()
        .map(|item| &item.data)
        .collect();
    assert_eq!(
        pre_payloads,
        vec![
            &PreLayoutOp::BankLease(
                BankLeaseSpec::new(lease, MbcBankClass::Rom, 12).expect("lease")
            ),
            &PreLayoutOp::AssertBank {
                expected: MbcBankClass::Rom,
                expected_n: 12,
            },
            &PreLayoutOp::TraceProbe {
                id: TraceProbeId::new(3),
                level: ProbeLevel::Debug,
            },
            &PreLayoutOp::Yield {
                kind: YieldKind::PollInterrupts,
            },
            &PreLayoutOp::BankRelease { lease_id: lease },
        ]
    );
    assert_eq!(
        &section.legalization_ops()[0].data,
        &LegalizationOp::FarCall {
            target,
            lease_chain: vec![lease],
        }
    );
    // Authoring order is preserved across the SoA via seq_index.
    let seqs: Vec<u32> = section
        .iter_items()
        .iter()
        .map(|item| item.seq_index())
        .collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(section.fixed_item_bytes(), None);
}

#[cfg(test)]
#[test]
fn builder_rejects_invalid_alignment_and_duplicate_labels() {
    let mut builder = Builder::new(
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("boot", "labels").expect("section name"),
    );
    let label = SymbolName::runtime("boot", "entry").expect("label");

    assert_eq!(builder.try_align(0), Err(BuilderError::ZeroAlignment));
    assert_eq!(
        builder.try_label(label.clone()).expect("first label").get(),
        0
    );
    assert_eq!(
        builder.try_label(label.clone()),
        Err(BuilderError::DuplicateLabel { name: label })
    );
}

#[cfg(test)]
#[test]
fn builder_validates_lease_lifecycle_and_bank_ranges() {
    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::runtime("banking", "validation").expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    let lease = LeaseId::new(4);

    assert!(BankLeaseSpec::new(lease, MbcBankClass::Rom, 512).is_err());
    assert!(BankLeaseSpec::new(lease, MbcBankClass::Sram, 16).is_err());
    assert_eq!(
        builder.try_bank_release(lease),
        Err(BuilderError::UnknownLease { lease_id: lease })
    );
    assert_eq!(
        builder.try_far_call(
            SymbolName::runtime("expert", "enter").expect("target"),
            &[lease]
        ),
        Err(BuilderError::UnknownLease { lease_id: lease })
    );
    assert_eq!(
        builder.try_assert_bank(MbcBankClass::Sram, 16),
        Err(BuilderError::SramBankOutOfRange { bank: 16 })
    );
    builder
        .try_assert_bank(MbcBankClass::Rom, 511)
        .expect("MBC5 ROM bank 511 is representable");
    assert_eq!(
        builder.try_assert_bank(MbcBankClass::Rom, 512),
        Err(BuilderError::RomBankOutOfRange { bank: 512 })
    );

    let spec = BankLeaseSpec::new(lease, MbcBankClass::Sram, 15).expect("valid sram lease");
    builder.try_bank_lease(spec.clone()).expect("lease");
    assert_eq!(
        builder.try_bank_lease(spec),
        Err(BuilderError::DuplicateLease { lease_id: lease })
    );
    builder
        .try_far_call(
            SymbolName::runtime("expert", "enter").expect("target"),
            &[lease],
        )
        .expect("far call through active lease");
    builder.try_bank_release(lease).expect("release");
    assert_eq!(
        builder.try_far_call(
            SymbolName::runtime("expert", "enter").expect("target"),
            &[lease]
        ),
        Err(BuilderError::UnknownLease { lease_id: lease })
    );
}

#[cfg(test)]
#[test]
fn builder_rejects_privileged_effects_in_normal_sections() {
    use crate::effect::{MbcRegisterClass, PrivilegeClass};
    use crate::isa::DirectAddr;

    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("normal_privilege", 0).expect("section name"),
    );
    let addr = DirectAddr::new(0x2000).expect("mbc register address");
    assert_eq!(
        builder.try_emit(Instr::LdDirectFromA { addr }),
        Err(BuilderError::PrivilegeViolation {
            effect: MachineEffect::StoreToMbcRegister {
                reg: MbcRegisterClass::RomBankLow,
            },
            violation: PrivilegeViolation::RequiredPrivilege {
                required: PrivilegeClass::Privileged,
                section: PrivilegeClass::Normal,
            },
        })
    );

    builder.set_section_privilege(SectionPrivilege::privileged());
    builder
        .try_emit(Instr::LdDirectFromA { addr })
        .expect("privileged section accepts mbc write");
    let section = builder.finish();
    assert_eq!(section.total_items(), 1);
    assert_eq!(section.instrs().len(), 1);
}

#[cfg(test)]
#[test]
fn builder_revalidates_existing_items_when_privilege_changes() {
    use crate::effect::{MbcRegisterClass, PrivilegeClass};
    use crate::isa::DirectAddr;

    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("privilege_downgrade", 0).expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    let addr = DirectAddr::new(0x2000).expect("mbc register address");
    builder
        .try_emit(Instr::LdDirectFromA { addr })
        .expect("privileged section accepts mbc write");

    assert_eq!(
        builder.try_set_section_privilege(SectionPrivilege::normal()),
        Err(BuilderError::SectionPrivilegeViolation(
            crate::section::SectionPrivilegeError {
                seq_index: 0,
                effect: MachineEffect::StoreToMbcRegister {
                    reg: MbcRegisterClass::RomBankLow,
                },
                violation: PrivilegeViolation::RequiredPrivilege {
                    required: PrivilegeClass::Privileged,
                    section: PrivilegeClass::Normal,
                },
            }
        ))
    );
}

#[cfg(test)]
#[test]
fn structured_ops_run_in_normal_sections() {
    // Resolved F-A1 decision: BankLease/BankRelease/AssertBank are Normal.
    // The privileged work happens inside the runtime helper; the call site
    // does not need a Privileged section.
    use crate::section::{BranchKind, SymbolicBranch};

    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::runtime("banking", "normal").expect("section name"),
    );
    let lease = LeaseId::new(11);
    let target = SymbolName::runtime("expert", "enter").expect("target");

    builder
        .try_bank_lease(BankLeaseSpec::new(lease, MbcBankClass::Rom, 7).expect("lease"))
        .expect("bank lease in Normal section");
    builder
        .try_assert_bank(MbcBankClass::Rom, 7)
        .expect("assert bank in Normal section");
    builder
        .try_far_call(target.clone(), &[lease])
        .expect("far call in Normal section");
    builder
        .try_branch(SymbolicBranch::new(BranchKind::Jump, None, target.clone()))
        .expect("symbolic branch in Normal section");
    builder
        .try_bank_release(lease)
        .expect("bank release in Normal section");

    let section = builder.finish();
    assert_eq!(section.total_items(), 5);
    assert_eq!(section.pre_layout_ops().len(), 3);
    assert_eq!(section.legalization_ops().len(), 1);
    assert_eq!(section.branches().len(), 1);
}

#[cfg(test)]
#[test]
fn builder_emits_symbolic_branches() {
    use crate::isa::Cond;
    use crate::section::{BranchKind, SymbolicBranch};

    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::runtime("loop", "section").expect("section name"),
    );
    let head = SymbolName::runtime("loop", "head").expect("head label");
    let target = SymbolName::runtime("expert", "enter").expect("call target");

    builder.label(head.clone());
    builder.branch(SymbolicBranch::new(BranchKind::Jump, None, head.clone()));
    builder.branch(SymbolicBranch::new(
        BranchKind::Jump,
        Some(Cond::NZ),
        head.clone(),
    ));
    builder.branch(SymbolicBranch::new(BranchKind::Call, None, target.clone()));

    let section = builder.finish();
    assert_eq!(section.total_items(), 4);
    assert_eq!(section.labels().len(), 1);
    assert_eq!(section.branches().len(), 3);

    let effects: Vec<MachineEffect> = section
        .branches()
        .iter()
        .map(|item| item.data.machine_effect())
        .collect();
    assert_eq!(
        effects,
        vec![
            MachineEffect::UnconditionalBranch,
            MachineEffect::ConditionalBranch,
            MachineEffect::Call,
        ]
    );
    assert_eq!(section.fixed_item_bytes(), None);
}

#[cfg(test)]
#[test]
fn builder_forbids_reserved_mbc_writes_in_every_section() {
    use crate::isa::DirectAddr;

    let addr = DirectAddr::new(0x6000).expect("reserved mbc address");
    let reserved_effect = MachineEffect::StoreToMbcRegister {
        reg: crate::effect::MbcRegisterClass::Reserved,
    };

    let mut normal = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("reserved_normal", 0).expect("section name"),
    );
    assert_eq!(
        normal.try_emit(Instr::LdDirectFromA { addr }),
        Err(BuilderError::PrivilegeViolation {
            effect: reserved_effect,
            violation: PrivilegeViolation::ForbiddenMbcReserved,
        })
    );

    let mut privileged = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("reserved_privileged", 0).expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    assert_eq!(
        privileged.try_emit(Instr::LdDirectFromA { addr }),
        Err(BuilderError::PrivilegeViolation {
            effect: reserved_effect,
            violation: PrivilegeViolation::ForbiddenMbcReserved,
        })
    );
}

#[cfg(test)]
#[test]
fn db_dw_rejected_in_executable_sections() {
    // Closes the raw-byte privilege loophole: an author cannot hand-encode
    // privileged instruction bytes (e.g. `LD ($2000), A` as `db 0xEA, 0x00,
    // 0x20`) in an executable section to slip past the effect classifier.
    // SectionRole::permits_inline_data() is the gate.
    for role in [
        SectionRole::Bank0Nucleus,
        SectionRole::CommonBank,
        SectionRole::ExpertBank,
    ] {
        let mut builder = Builder::new(
            role,
            SymbolName::section(role, SectionId::new(0)).expect("section name"),
        );
        // Even a privileged section cannot relax this — privilege is for
        // typed effects (Instr, structured ops); inline bytes have no typed
        // effect to check, so the only safe gate is the role.
        if matches!(role, SectionRole::CommonBank) {
            builder.set_section_privilege(SectionPrivilege::privileged());
        }

        assert_eq!(
            builder.try_db(0xEA),
            Err(BuilderError::InlineDataInExecutableSection { role })
        );
        assert_eq!(
            builder.try_db_bytes([0xEA, 0x00, 0x20]),
            Err(BuilderError::InlineDataInExecutableSection { role })
        );
        assert_eq!(
            builder.try_dw(0x20EA),
            Err(BuilderError::InlineDataInExecutableSection { role })
        );
        assert_eq!(
            builder.try_dw_words([0x20EA, 0x1234]),
            Err(BuilderError::InlineDataInExecutableSection { role })
        );
        assert_eq!(builder.finish().total_items(), 0);
    }

    // Data-only roles accept inline data unconditionally.
    for role in [
        SectionRole::HeaderCartridge,
        SectionRole::WramHotArena,
        SectionRole::WramOverlay,
        SectionRole::HramFastFlags,
        SectionRole::SramPersistent,
        SectionRole::VramOwnedByUi,
        SectionRole::OamOwnedByUi,
    ] {
        let mut builder = Builder::new(
            role,
            SymbolName::section(role, SectionId::new(0)).expect("section name"),
        );
        builder.try_db(0x42).expect("data role accepts db");
        builder
            .try_dw_words([0x1234, 0x5678])
            .expect("data role accepts dw_words");
        let section = builder.finish();
        assert_eq!(section.data_blocks().len(), 2);
    }
}

#[cfg(test)]
#[test]
fn provenance_scope_restores_after_caught_panic() {
    let default_provenance =
        InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default");
    let temporary = InstrProvenance::new(PlanningStage::ArenaPlan).with_source_op("temp_scope");
    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("panic_scope", 0).expect("section name"),
    );

    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        builder.with_provenance(temporary, |_| panic!("synthetic panic"));
    }));
    assert!(panic_result.is_err());

    builder.emit(Instr::Nop);
    let section = builder.finish();

    assert_eq!(section.instrs()[0].provenance, default_provenance);
}
