//! Structured-op lowering seams.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::isa::Instr;
use crate::provenance::InstrProvenance;
use crate::section::{
    Align, DataBlock, Label, LegalizationOp, LoweredSection, MbcBankClass, OrderedItem,
    PreLayoutOp, PrivilegeViolation, ProbeLevel, Section, SectionId, SectionPrivilege,
    SectionPrivilegeError, SectionRole, SymbolicBranch, TraceProbeId, YieldKind,
};
use crate::symbols::{SymbolName, SymbolTable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringError {
    UnsupportedStructuredOp(&'static str),
    SymbolName(String),
    SectionPrivilegeViolation(SectionPrivilegeError),
}

impl fmt::Display for LoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedStructuredOp(op) => write!(f, "unsupported structured op {op}"),
            Self::SymbolName(message) => write!(f, "invalid generated symbol name: {message}"),
            Self::SectionPrivilegeViolation(error) => {
                write!(f, "lowered fragment violates section privilege: {error:?}")
            }
        }
    }
}

impl std::error::Error for LoweringError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweringContext<'a> {
    pub source_section_id: SectionId,
    pub source_section_role: SectionRole,
    pub provenance: &'a InstrProvenance,
    pub symbols: &'a SymbolTable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentItem<T> {
    pub data: T,
    pub sub_index: u16,
    pub provenance: InstrProvenance,
}

impl<T> FragmentItem<T> {
    #[must_use]
    pub fn new(data: T, provenance: InstrProvenance) -> Self {
        Self::new_with_sub_index(data, 0, provenance)
    }

    #[must_use]
    pub fn new_with_sub_index(data: T, sub_index: u16, provenance: InstrProvenance) -> Self {
        Self {
            data,
            sub_index,
            provenance,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoweredFragment {
    pub labels: Vec<FragmentItem<Label>>,
    pub instrs: Vec<FragmentItem<Instr>>,
    pub data_blocks: Vec<FragmentItem<DataBlock>>,
    pub alignments: Vec<FragmentItem<Align>>,
    pub legalization_ops: Vec<FragmentItem<LegalizationOp>>,
    pub branches: Vec<FragmentItem<SymbolicBranch>>,
}

pub trait PreLayoutOpLowering {
    fn lower(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
    ) -> Result<LoweredFragment, LoweringError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceLoweringPolicy {
    EmitCalls,
    Elide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertBankLoweringPolicy {
    EmitRuntimeCheck,
    Elide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StubLoweringConfig {
    pub trace_policy: TraceLoweringPolicy,
    pub assert_bank_policy: AssertBankLoweringPolicy,
}

impl Default for StubLoweringConfig {
    fn default() -> Self {
        Self {
            trace_policy: TraceLoweringPolicy::Elide,
            assert_bank_policy: AssertBankLoweringPolicy::Elide,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubPreLayoutOpLowering {
    pub config: StubLoweringConfig,
}

impl PreLayoutOpLowering for StubPreLayoutOpLowering {
    fn lower(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
    ) -> Result<LoweredFragment, LoweringError> {
        match op {
            PreLayoutOp::BankLease(spec) => branch_fragment(
                runtime_symbol(&format!(
                    "lease_{}_{}",
                    bank_class_name(spec.class()),
                    spec.bank()
                ))?,
                ctx.provenance.clone(),
            ),
            PreLayoutOp::BankRelease { lease_id } => branch_fragment(
                runtime_symbol(&format!("release_{}", lease_id.get()))?,
                ctx.provenance.clone(),
            ),
            PreLayoutOp::Yield { kind } => {
                branch_fragment(runtime_symbol(yield_symbol(*kind))?, ctx.provenance.clone())
            }
            PreLayoutOp::TraceProbe { id, level } => match self.config.trace_policy {
                TraceLoweringPolicy::EmitCalls => branch_fragment(
                    runtime_symbol(&trace_symbol(*id, *level))?,
                    ctx.provenance.clone(),
                ),
                TraceLoweringPolicy::Elide => Ok(LoweredFragment::default()),
            },
            PreLayoutOp::AssertBank {
                expected,
                expected_n,
            } => match self.config.assert_bank_policy {
                AssertBankLoweringPolicy::EmitRuntimeCheck => branch_fragment(
                    runtime_symbol(&format!(
                        "assert_{}_{}",
                        bank_class_name(*expected),
                        expected_n
                    ))?,
                    ctx.provenance.clone(),
                ),
                AssertBankLoweringPolicy::Elide => Ok(LoweredFragment::default()),
            },
        }
    }
}

pub fn lower_pre_layout_ops(
    sections: Vec<Section>,
    lowerer: &dyn PreLayoutOpLowering,
    symbols: &SymbolTable,
) -> Result<Vec<LoweredSection>, LoweringError> {
    sections
        .into_iter()
        .map(|section| lower_one(section, lowerer, symbols))
        .collect()
}

fn lower_one(
    section: Section,
    lowerer: &dyn PreLayoutOpLowering,
    symbols: &SymbolTable,
) -> Result<LoweredSection, LoweringError> {
    let mut labels = section.labels().to_vec();
    let mut instrs = section.instrs().to_vec();
    let mut data_blocks = section.data_blocks().to_vec();
    let mut alignments = section.alignments().to_vec();
    let mut legalization_ops = section.legalization_ops().to_vec();
    let mut branches = section.branches().to_vec();

    for op in section.pre_layout_ops() {
        let ctx = LoweringContext {
            source_section_id: section.id(),
            source_section_role: section.role(),
            provenance: &op.provenance,
            symbols,
        };
        let fragment = lowerer.lower(&op.data, &ctx)?;
        validate_fragment(&fragment, section.privilege(), op.seq_index)?;
        labels.extend(fragment.labels.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
        instrs.extend(fragment.instrs.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
        data_blocks.extend(fragment.data_blocks.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
        alignments.extend(fragment.alignments.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
        legalization_ops.extend(fragment.legalization_ops.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
        branches.extend(fragment.branches.into_iter().map(|item| {
            OrderedItem::new_with_sub_index(
                item.data,
                op.seq_index,
                item.sub_index,
                item.provenance,
            )
        }));
    }

    Ok(LoweredSection {
        id: section.id(),
        role: section.role(),
        name: section.name().clone(),
        privilege: section.privilege().clone(),
        align: section.align(),
        size_hint_bytes: section.size_hint_bytes(),
        next_seq_index: next_seq_index_after(
            &labels,
            &instrs,
            &data_blocks,
            &alignments,
            &legalization_ops,
            &branches,
        ),
        labels,
        instrs,
        data_blocks,
        alignments,
        legalization_ops,
        branches,
    })
}

fn next_seq_index_after(
    labels: &[OrderedItem<Label>],
    instrs: &[OrderedItem<Instr>],
    data_blocks: &[OrderedItem<DataBlock>],
    alignments: &[OrderedItem<Align>],
    legalization_ops: &[OrderedItem<LegalizationOp>],
    branches: &[OrderedItem<SymbolicBranch>],
) -> u32 {
    labels
        .iter()
        .map(|item| item.seq_index)
        .chain(instrs.iter().map(|item| item.seq_index))
        .chain(data_blocks.iter().map(|item| item.seq_index))
        .chain(alignments.iter().map(|item| item.seq_index))
        .chain(legalization_ops.iter().map(|item| item.seq_index))
        .chain(branches.iter().map(|item| item.seq_index))
        .max()
        .map_or(0, |max| max.saturating_add(1))
}

fn branch_fragment(
    target: SymbolName,
    provenance: InstrProvenance,
) -> Result<LoweredFragment, LoweringError> {
    Ok(LoweredFragment {
        branches: vec![FragmentItem::new(
            SymbolicBranch::call(target, None),
            provenance,
        )],
        ..LoweredFragment::default()
    })
}

fn validate_fragment(
    fragment: &LoweredFragment,
    privilege: &SectionPrivilege,
    seq_index: u32,
) -> Result<(), LoweringError> {
    for item in &fragment.instrs {
        let effect = crate::effect::classify_effect(&item.data);
        privilege
            .check_effect(effect)
            .map_err(|violation| lowering_privilege_error(seq_index, effect, violation))?;
    }
    for item in &fragment.legalization_ops {
        let effect = crate::effect::classify_legalization_op(&item.data);
        privilege
            .check_effect(effect)
            .map_err(|violation| lowering_privilege_error(seq_index, effect, violation))?;
    }
    for item in &fragment.branches {
        let effect = item.data.machine_effect();
        privilege
            .check_effect(effect)
            .map_err(|violation| lowering_privilege_error(seq_index, effect, violation))?;
    }
    Ok(())
}

fn lowering_privilege_error(
    seq_index: u32,
    effect: crate::effect::MachineEffect,
    violation: PrivilegeViolation,
) -> LoweringError {
    LoweringError::SectionPrivilegeViolation(SectionPrivilegeError {
        seq_index,
        effect,
        violation,
    })
}

fn runtime_symbol(symbol: &str) -> Result<SymbolName, LoweringError> {
    SymbolName::runtime("stub_runtime", symbol)
        .map_err(|err| LoweringError::SymbolName(err.to_string()))
}

const fn bank_class_name(class: MbcBankClass) -> &'static str {
    match class {
        MbcBankClass::Rom => "rom",
        MbcBankClass::Sram => "sram",
    }
}

const fn yield_symbol(kind: YieldKind) -> &'static str {
    match kind {
        YieldKind::PollInterrupts => "yield_poll_interrupts",
        YieldKind::FrameBoundary => "yield_frame_boundary",
        YieldKind::Cooperative => "yield_cooperative",
    }
}

fn trace_symbol(id: TraceProbeId, level: ProbeLevel) -> String {
    let level = match level {
        ProbeLevel::Trace => "trace",
        ProbeLevel::Debug => "debug",
        ProbeLevel::Info => "info",
    };
    format!("trace_{}_{}", level, id.get())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;
    use crate::builder::Builder;
    use crate::isa::DirectAddr;
    use crate::section::{BankLeaseSpec, Label, LeaseId, MbcBankClass, SectionRole, SymbolId};

    #[test]
    fn pre_layout_ops_are_drained() {
        let mut builder = Builder::new(
            SectionRole::CommonBank,
            SymbolName::runtime("demo", "ops").expect("section"),
        );
        let lease = LeaseId::new(1);
        builder.bank_lease(BankLeaseSpec::new(lease, MbcBankClass::Rom, 2).expect("valid lease"));
        builder.yield_op(YieldKind::Cooperative);
        builder.bank_release(lease);

        let lowered = lower_pre_layout_ops(
            vec![builder.finish()],
            &StubPreLayoutOpLowering {
                config: StubLoweringConfig {
                    trace_policy: TraceLoweringPolicy::Elide,
                    assert_bank_policy: AssertBankLoweringPolicy::Elide,
                },
            },
            &SymbolTable::new(),
        )
        .expect("lowering succeeds");

        assert_eq!(lowered.len(), 1);
        assert_eq!(lowered[0].branches.len(), 3);
    }

    #[test]
    fn elided_stub_ops_emit_no_items() {
        let mut builder = Builder::new(
            SectionRole::CommonBank,
            SymbolName::runtime("demo", "trace").expect("section"),
        );
        builder.trace_probe(TraceProbeId::new(7), ProbeLevel::Info);
        builder.assert_bank(MbcBankClass::Rom, 1);

        let lowered = lower_pre_layout_ops(
            vec![builder.finish()],
            &StubPreLayoutOpLowering::default(),
            &SymbolTable::new(),
        )
        .expect("lowering succeeds");

        assert!(lowered[0].branches.is_empty());
        assert_eq!(lowered[0].align, NonZeroU16::new(1).expect("nonzero"));
    }

    struct PrivilegedLowerer;

    impl PreLayoutOpLowering for PrivilegedLowerer {
        fn lower(
            &self,
            _op: &PreLayoutOp,
            ctx: &LoweringContext<'_>,
        ) -> Result<LoweredFragment, LoweringError> {
            Ok(LoweredFragment {
                instrs: vec![FragmentItem::new(
                    Instr::LdDirectFromA {
                        addr: DirectAddr::new(0x2000).expect("mbc register"),
                    },
                    ctx.provenance.clone(),
                )],
                ..LoweredFragment::default()
            })
        }
    }

    #[test]
    fn lowered_fragments_are_revalidated_against_section_privilege() {
        let mut builder = Builder::new(
            SectionRole::CommonBank,
            SymbolName::runtime("demo", "privilege").expect("section"),
        );
        builder.yield_op(YieldKind::Cooperative);

        let err = lower_pre_layout_ops(
            vec![builder.finish()],
            &PrivilegedLowerer,
            &SymbolTable::new(),
        )
        .expect_err("privileged lowered instruction rejected");

        assert!(matches!(
            err,
            LoweringError::SectionPrivilegeViolation(SectionPrivilegeError { .. })
        ));
    }

    struct OrderedFragmentLowerer;

    impl PreLayoutOpLowering for OrderedFragmentLowerer {
        fn lower(
            &self,
            _op: &PreLayoutOp,
            ctx: &LoweringContext<'_>,
        ) -> Result<LoweredFragment, LoweringError> {
            Ok(LoweredFragment {
                instrs: vec![FragmentItem::new_with_sub_index(
                    Instr::Nop,
                    0,
                    ctx.provenance.clone(),
                )],
                labels: vec![FragmentItem::new_with_sub_index(
                    Label {
                        id: SymbolId::new(0),
                        name: SymbolName::runtime("demo", "after_lowered_nop").expect("label"),
                    },
                    1,
                    ctx.provenance.clone(),
                )],
                ..LoweredFragment::default()
            })
        }
    }

    #[test]
    fn lowered_fragments_preserve_sub_index_order() {
        let mut builder = Builder::new(
            SectionRole::CommonBank,
            SymbolName::runtime("demo", "order").expect("section"),
        );
        builder.yield_op(YieldKind::Cooperative);

        let lowered = lower_pre_layout_ops(
            vec![builder.finish()],
            &OrderedFragmentLowerer,
            &SymbolTable::new(),
        )
        .expect("lowering succeeds");

        assert!(lowered[0].instrs[0].order() < lowered[0].labels[0].order());
    }
}
