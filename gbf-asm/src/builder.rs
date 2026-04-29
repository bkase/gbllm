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
    BankLeaseSpec, LeaseId, LegalizationOp, MbcBankClass, PreLayoutOp, PrivilegeViolation,
    ProbeLevel, Section, SectionId, SectionItem, SectionPrivilege, SectionPrivilegeError,
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
        bank: u8,
    },
    PrivilegeViolation {
        effect: MachineEffect,
        violation: PrivilegeViolation,
    },
    SectionPrivilegeViolation(SectionPrivilegeError),
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
            Self::PrivilegeViolation { effect, violation } => {
                write!(
                    f,
                    "section privilege rejected effect {effect:?}: {violation:?}"
                )
            }
            Self::SectionPrivilegeViolation(error) => {
                write!(
                    f,
                    "section privilege rejected existing item {} with effect {:?}: {:?}",
                    error.item_index, error.effect, error.violation
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
        self.section
            .push(SectionItem::instr(instr, self.cur_provenance.clone()));
        Ok(())
    }

    pub fn db(&mut self, byte: u8) {
        self.db_bytes([byte]);
    }

    pub fn db_bytes(&mut self, bytes: impl Into<Vec<u8>>) {
        self.section
            .push(SectionItem::db(bytes, self.cur_provenance.clone()));
    }

    pub fn dw(&mut self, word: u16) {
        self.dw_words([word]);
    }

    pub fn dw_words(&mut self, words: impl Into<Vec<u16>>) {
        self.section
            .push(SectionItem::dw(words, self.cur_provenance.clone()));
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
            .push(SectionItem::label(id, name, self.cur_provenance.clone()));
        Ok(id)
    }

    pub fn align(&mut self, align: NonZeroU16) {
        self.section
            .push(SectionItem::align(align, self.cur_provenance.clone()));
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
            .push(SectionItem::branch(branch, self.cur_provenance.clone()));
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

    pub fn assert_bank(&mut self, expected: MbcBankClass, expected_n: u8) {
        self.try_assert_bank(expected, expected_n)
            .expect("invalid bank assertion in Builder::assert_bank");
    }

    pub fn try_assert_bank(
        &mut self,
        expected: MbcBankClass,
        expected_n: u8,
    ) -> Result<(), BuilderError> {
        let op = PreLayoutOp::AssertBank {
            expected,
            expected_n,
        };
        self.validate_effect(classify_pre_layout_op(&op))?;
        if expected == MbcBankClass::Sram && u16::from(expected_n) > BankLeaseSpec::MAX_SRAM_BANK {
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
            .push(SectionItem::pre_layout_op(op, self.cur_provenance.clone()));
    }

    fn legalization_op_unchecked(&mut self, op: LegalizationOp) {
        self.section.push(SectionItem::legalization_op(
            op,
            self.cur_provenance.clone(),
        ));
    }
}

#[cfg(test)]
#[test]
fn roundtrip() {
    use crate::isa::Reg8;

    let mut builder = Builder::new(
        SectionRole::Bank0Nucleus,
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

    assert_eq!(section.role(), SectionRole::Bank0Nucleus);
    assert_eq!(section.items().len(), 5);
    assert_eq!(section.fixed_item_bytes(), None);
    assert_eq!(entry.get(), 0);
    assert_eq!(
        section.items()[0],
        SectionItem::label(
            entry,
            SymbolName::runtime("boot", "entry").expect("label"),
            InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
        )
    );
    assert_eq!(
        section.items()[1],
        SectionItem::instr(
            Instr::Ld8RegFromImm {
                dst: Reg8::A,
                imm: 0x12,
            },
            InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
        )
    );
    assert_eq!(
        section.items()[2],
        SectionItem::db(
            [0x34],
            InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
        )
    );
    assert_eq!(
        section.items()[3],
        SectionItem::dw(
            [0x5678],
            InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
        )
    );
}

#[cfg(test)]
#[test]
fn provenance_recorded() {
    let default_provenance =
        InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default");
    let storage_provenance =
        InstrProvenance::new(PlanningStage::StoragePlan).with_source_op("arena_bind");

    let mut builder = Builder::new(
        SectionRole::CommonBank,
        SymbolName::kernel("copy", 1).expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.emit(Instr::Nop);
    builder.with_provenance(storage_provenance.clone(), |builder| {
        builder.db(0xAA);
        builder.db_bytes([0xBB, 0xCC]);
    });
    builder.dw(0x1234);

    let section = builder.finish();

    assert_eq!(section.items()[0].provenance(), &default_provenance);
    assert_eq!(section.items()[1].provenance(), &storage_provenance);
    assert_eq!(section.items()[2].provenance(), &storage_provenance);
    assert_eq!(section.items()[3].provenance(), &default_provenance);
    assert!(
        section
            .items()
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

    assert_eq!(section.items().len(), 6);
    assert_eq!(
        section.items(),
        [
            SectionItem::pre_layout_op(
                PreLayoutOp::BankLease(
                    BankLeaseSpec::new(lease, MbcBankClass::Rom, 12).expect("lease")
                ),
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
            SectionItem::pre_layout_op(
                PreLayoutOp::AssertBank {
                    expected: MbcBankClass::Rom,
                    expected_n: 12,
                },
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
            SectionItem::pre_layout_op(
                PreLayoutOp::TraceProbe {
                    id: TraceProbeId::new(3),
                    level: ProbeLevel::Debug,
                },
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
            SectionItem::pre_layout_op(
                PreLayoutOp::Yield {
                    kind: YieldKind::PollInterrupts,
                },
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
            SectionItem::legalization_op(
                LegalizationOp::FarCall {
                    target,
                    lease_chain: vec![lease],
                },
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
            SectionItem::pre_layout_op(
                PreLayoutOp::BankRelease { lease_id: lease },
                InstrProvenance::new(PlanningStage::Backend).with_source_op("builder.default")
            ),
        ]
    );
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
    assert_eq!(builder.finish().items().len(), 1);
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
                item_index: 0,
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

    assert_eq!(builder.finish().items().len(), 5);
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
    assert_eq!(section.items().len(), 4);
    assert_eq!(
        section.items()[1].machine_effect(),
        Some(MachineEffect::UnconditionalBranch)
    );
    assert_eq!(
        section.items()[2].machine_effect(),
        Some(MachineEffect::ConditionalBranch)
    );
    assert_eq!(
        section.items()[3].machine_effect(),
        Some(MachineEffect::Call)
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

    assert_eq!(section.items()[0].provenance(), &default_provenance);
}
