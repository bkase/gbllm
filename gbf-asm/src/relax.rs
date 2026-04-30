//! Branch relaxation and legalization.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::isa::{Instr, Reg8, Reg16Data};
use crate::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};
use crate::provenance::{InstrProvenance, PlanningStage};
use crate::section::{
    BranchKind, DataBlock, Label, LegalizationOp, LegalizedSection, LoweredSection, OrderedItem,
    SectionId, SectionRole, SymbolicBranch,
};
use crate::symbols::{SymbolAddress, SymbolName, SymbolTable, SymbolTableError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaxedProgram {
    pub sections: Vec<LegalizedSection>,
    pub layout: LayoutPlan,
    pub symbols: SymbolTable,
    pub thunk_requests: Vec<ResolvedThunkRequest>,
    pub iterations: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedThunkRequest {
    pub thunk_symbol: SymbolName,
    pub target: SymbolName,
    pub callee_bank: BankIndex,
    pub target_cpu_addr: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaxError {
    NoFixedPoint {
        iters: u8,
    },
    MissingPlacement {
        section_id: SectionId,
    },
    MissingAlignmentPlan {
        section_id: SectionId,
        seq_index: u32,
    },
    UnresolvedSymbol {
        name: SymbolName,
        used_in: SectionId,
    },
    DuplicateSymbol(SymbolTableError),
    InvalidRelativeOffset {
        offset: i32,
    },
    CrossBankBranchUnsupported {
        used_in: SectionId,
        source_bank: BankIndex,
        target: SymbolName,
        target_bank: BankIndex,
    },
}

impl fmt::Display for RelaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFixedPoint { iters } => {
                write!(
                    f,
                    "branch relaxation did not reach a fixed point in {iters} iterations"
                )
            }
            Self::MissingPlacement { section_id } => {
                write!(f, "section {} has no placement", section_id.get())
            }
            Self::MissingAlignmentPlan {
                section_id,
                seq_index,
            } => write!(
                f,
                "section {} align item {seq_index} has no layout padding",
                section_id.get()
            ),
            Self::UnresolvedSymbol { name, used_in } => {
                write!(
                    f,
                    "symbol {name} used in section {} is unresolved",
                    used_in.get()
                )
            }
            Self::DuplicateSymbol(error) => write!(f, "{error}"),
            Self::InvalidRelativeOffset { offset } => {
                write!(f, "relative branch offset {offset} is outside i8 range")
            }
            Self::CrossBankBranchUnsupported {
                used_in,
                source_bank,
                target,
                target_bank,
            } => write!(
                f,
                "section {} in {source_bank} cannot branch directly to {target} in {target_bank}",
                used_in.get()
            ),
        }
    }
}

impl std::error::Error for RelaxError {}

pub fn relax_and_legalize(
    sections: &[LoweredSection],
    layout: &LayoutPlan,
) -> Result<RelaxedProgram, RelaxError> {
    let relaxable_branch_count: usize = sections
        .iter()
        .map(|section| {
            section
                .branches
                .iter()
                .filter(|branch| branch.data.kind == BranchKind::Jump)
                .count()
        })
        .sum();
    let hard_cap = 1 + relaxable_branch_count;
    let mut wide_jumps: BTreeMap<(SectionId, u32), bool> = BTreeMap::new();
    let mut iterations = 0_u8;

    for iter in 0..=hard_cap {
        iterations = (iter + 1) as u8;
        let symbols = build_symbol_table(sections, layout, &wide_jumps)?;
        let mut changed = false;
        for section in sections {
            let placed = placed_for(layout, section.id)?;
            let offsets = item_offsets(section, placed, &wide_jumps)?;
            for branch in &section.branches {
                if branch.data.kind != BranchKind::Jump {
                    continue;
                }
                let source_cpu = placed.cpu_start as i32 + offsets[&branch.seq_index] as i32;
                let target = resolve_target(&symbols, layout, &branch.data.target, section.id)?;
                ensure_directly_reachable(section.id, placed, &target, &branch.data.target)?;
                let delta = target.cpu_addr as i32 - (source_cpu + 2);
                if !(-128..=127).contains(&delta)
                    && !wide_jumps
                        .get(&(section.id, branch.seq_index))
                        .copied()
                        .unwrap_or(false)
                {
                    wide_jumps.insert((section.id, branch.seq_index), true);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
        if iter == hard_cap {
            return Err(RelaxError::NoFixedPoint { iters: iterations });
        }
    }

    let mut symbols = build_symbol_table(sections, layout, &wide_jumps)?;
    let mut final_layout = layout.clone();
    let mut thunk_by_target: BTreeMap<SymbolName, ResolvedThunkRequest> = BTreeMap::new();
    let mut thunk_order: Vec<SymbolName> = Vec::new();
    let mut legalized = Vec::with_capacity(sections.len());

    for section in sections {
        let placed = placed_for(&final_layout, section.id)?;
        let legal = legalize_section(
            section,
            placed,
            &final_layout,
            &symbols,
            &wide_jumps,
            &mut thunk_by_target,
            &mut thunk_order,
        )?;
        let final_size = legalized_size(&legal, placed)? as u16;
        if let Some(placed_mut) = final_layout
            .sections
            .iter_mut()
            .find(|candidate| candidate.id == section.id)
        {
            placed_mut.final_size = final_size;
        }
        legalized.push(legal);
    }

    let mut requests = Vec::with_capacity(thunk_order.len());
    for (idx, target) in thunk_order.iter().enumerate() {
        let request = thunk_by_target
            .get(target)
            .expect("thunk_order entries are present")
            .clone();
        let thunk_id = SectionId::new(0xF000 + idx as u32);
        let cpu_start = 0x3F00 + (idx as u16 * 0x10);
        symbols
            .insert(
                request.thunk_symbol.clone(),
                SymbolAddress::new(thunk_id, 0),
            )
            .map_err(RelaxError::DuplicateSymbol)?;
        legalized.push(materialize_stub_thunk(thunk_id, cpu_start, &request));
        final_layout.sections.push(PlacedSection {
            id: thunk_id,
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start,
            final_size: 10,
            estimated_size: 10,
            alignment_padding: BTreeMap::new(),
        });
        requests.push(request);
    }

    Ok(RelaxedProgram {
        sections: legalized,
        layout: final_layout,
        symbols,
        thunk_requests: requests,
        iterations,
    })
}

fn build_symbol_table(
    sections: &[LoweredSection],
    layout: &LayoutPlan,
    wide_jumps: &BTreeMap<(SectionId, u32), bool>,
) -> Result<SymbolTable, RelaxError> {
    let mut table = SymbolTable::new();
    for section in sections {
        let placed = placed_for(layout, section.id)?;
        let offsets = item_offsets(section, placed, wide_jumps)?;
        for label in &section.labels {
            table
                .insert(
                    label.data.name.clone(),
                    SymbolAddress::new(section.id, offsets[&label.seq_index]),
                )
                .map_err(RelaxError::DuplicateSymbol)?;
        }
    }
    Ok(table)
}

fn legalize_section(
    section: &LoweredSection,
    placed: &PlacedSection,
    layout: &LayoutPlan,
    symbols: &SymbolTable,
    wide_jumps: &BTreeMap<(SectionId, u32), bool>,
    thunk_by_target: &mut BTreeMap<SymbolName, ResolvedThunkRequest>,
    thunk_order: &mut Vec<SymbolName>,
) -> Result<LegalizedSection, RelaxError> {
    let offsets = item_offsets(section, placed, wide_jumps)?;
    let mut labels = section.labels.clone();
    let mut instrs = section.instrs.clone();
    let data_blocks = section.data_blocks.clone();
    let alignments = section.alignments.clone();

    let mut emitted = Vec::new();
    let branch_ctx = BranchLegalizationContext {
        used_in: section.id,
        placed,
        layout,
        symbols,
        wide_jumps,
    };
    for branch in &section.branches {
        emitted.push(OrderedItem::new(
            legalize_branch(
                &branch_ctx,
                &branch.data,
                branch.seq_index,
                offsets[&branch.seq_index],
            )?,
            branch.seq_index,
            branch.provenance.clone(),
        ));
    }

    for op in &section.legalization_ops {
        match &op.data {
            LegalizationOp::FarCall { target, .. } => {
                let target_resolved = resolve_target(symbols, layout, target, section.id)?;
                let call_addr = if directly_reachable(placed.bank, target_resolved.bank) {
                    target_resolved.cpu_addr
                } else {
                    if !thunk_by_target.contains_key(target) {
                        thunk_by_target.insert(
                            target.clone(),
                            ResolvedThunkRequest {
                                thunk_symbol: SymbolName::runtime_thunk_for(target)
                                    .expect("validated target segments produce thunk symbol"),
                                target: target.clone(),
                                callee_bank: target_resolved.bank,
                                target_cpu_addr: target_resolved.cpu_addr,
                            },
                        );
                        thunk_order.push(target.clone());
                    }
                    // The final thunk address is deterministic from insertion order.
                    let idx = thunk_order
                        .iter()
                        .position(|key| key == target)
                        .expect("inserted thunk target has an order");
                    0x3F00 + (idx as u16 * 0x10)
                };
                emitted.push(OrderedItem::new(
                    Instr::Call {
                        cond: None,
                        addr: call_addr,
                    },
                    op.seq_index,
                    op.provenance.clone(),
                ));
            }
        }
    }
    instrs.extend(emitted);
    labels.sort_by_key(|item| item.seq_index);
    instrs.sort_by_key(|item| item.seq_index);

    Ok(LegalizedSection {
        id: section.id,
        role: section.role,
        name: section.name.clone(),
        align: section.align,
        size_hint_bytes: section.size_hint_bytes,
        next_seq_index: section.next_seq_index,
        labels,
        instrs,
        data_blocks,
        alignments,
    })
}

struct BranchLegalizationContext<'a> {
    used_in: SectionId,
    placed: &'a PlacedSection,
    layout: &'a LayoutPlan,
    symbols: &'a SymbolTable,
    wide_jumps: &'a BTreeMap<(SectionId, u32), bool>,
}

fn legalize_branch(
    ctx: &BranchLegalizationContext<'_>,
    branch: &SymbolicBranch,
    seq_index: u32,
    offset: u32,
) -> Result<Instr, RelaxError> {
    let target = resolve_target(ctx.symbols, ctx.layout, &branch.target, ctx.used_in)?;
    ensure_directly_reachable(ctx.used_in, ctx.placed, &target, &branch.target)?;
    match branch.kind {
        BranchKind::Jump => {
            if ctx
                .wide_jumps
                .get(&(ctx.used_in, seq_index))
                .copied()
                .unwrap_or(false)
            {
                Ok(Instr::JpAbs {
                    cond: branch.cond,
                    addr: target.cpu_addr,
                })
            } else {
                let here = i32::from(ctx.placed.cpu_start) + offset as i32;
                let delta = i32::from(target.cpu_addr) - (here + 2);
                let off = i8::try_from(delta)
                    .map_err(|_| RelaxError::InvalidRelativeOffset { offset: delta })?;
                Ok(Instr::JrRel {
                    cond: branch.cond,
                    off,
                })
            }
        }
        BranchKind::Call => Ok(Instr::Call {
            cond: branch.cond,
            addr: target.cpu_addr,
        }),
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedTarget {
    bank: BankIndex,
    cpu_addr: u16,
}

fn resolve_target(
    symbols: &SymbolTable,
    layout: &LayoutPlan,
    target: &SymbolName,
    used_in: SectionId,
) -> Result<ResolvedTarget, RelaxError> {
    let address = symbols
        .resolve(target)
        .ok_or_else(|| RelaxError::UnresolvedSymbol {
            name: target.clone(),
            used_in,
        })?;
    let placed = placed_for(layout, address.section)?;
    Ok(ResolvedTarget {
        bank: placed.bank,
        cpu_addr: placed.cpu_start + address.offset as u16,
    })
}

fn ensure_directly_reachable(
    used_in: SectionId,
    source: &PlacedSection,
    target: &ResolvedTarget,
    target_name: &SymbolName,
) -> Result<(), RelaxError> {
    if directly_reachable(source.bank, target.bank) {
        Ok(())
    } else {
        Err(RelaxError::CrossBankBranchUnsupported {
            used_in,
            source_bank: source.bank,
            target: target_name.clone(),
            target_bank: target.bank,
        })
    }
}

fn directly_reachable(source: BankIndex, target: BankIndex) -> bool {
    target == BankIndex::Rom(0) || source == target
}

fn placed_for(layout: &LayoutPlan, section_id: SectionId) -> Result<&PlacedSection, RelaxError> {
    layout
        .sections
        .iter()
        .find(|placed| placed.id == section_id)
        .ok_or(RelaxError::MissingPlacement { section_id })
}

fn item_offsets(
    section: &LoweredSection,
    placed: &PlacedSection,
    wide_jumps: &BTreeMap<(SectionId, u32), bool>,
) -> Result<BTreeMap<u32, u32>, RelaxError> {
    let mut cursor = 0_u32;
    let mut offsets = BTreeMap::new();
    let mut items = Vec::new();
    items.extend(
        section
            .labels
            .iter()
            .map(|item| (item.seq_index, OffsetItem::Label)),
    );
    items.extend(section.instrs.iter().map(|item| {
        (
            item.seq_index,
            OffsetItem::Fixed(u32::from(item.data.byte_len())),
        )
    }));
    items.extend(section.data_blocks.iter().map(|item| {
        (
            item.seq_index,
            OffsetItem::Fixed(match &item.data {
                DataBlock::Bytes(bytes) => bytes.len() as u32,
                DataBlock::Words(words) => words.len() as u32 * 2,
            }),
        )
    }));
    items.extend(section.alignments.iter().map(|item| {
        (
            item.seq_index,
            OffsetItem::Align(placed.alignment_padding.get(&item.seq_index).copied()),
        )
    }));
    items.extend(section.branches.iter().map(|item| {
        let size = match item.data.kind {
            BranchKind::Jump => {
                if wide_jumps
                    .get(&(section.id, item.seq_index))
                    .copied()
                    .unwrap_or(false)
                {
                    3
                } else {
                    2
                }
            }
            BranchKind::Call => 3,
        };
        (item.seq_index, OffsetItem::Fixed(size))
    }));
    items.extend(
        section
            .legalization_ops
            .iter()
            .map(|item| (item.seq_index, OffsetItem::Fixed(3))),
    );
    items.sort_by_key(|(seq_index, _)| *seq_index);
    for (seq_index, item) in items {
        offsets.insert(seq_index, cursor);
        match item {
            OffsetItem::Label => {}
            OffsetItem::Fixed(size) => cursor += size,
            OffsetItem::Align(padding) => {
                let padding = padding.ok_or(RelaxError::MissingAlignmentPlan {
                    section_id: section.id,
                    seq_index,
                })?;
                cursor += u32::from(padding);
            }
        }
    }
    Ok(offsets)
}

fn legalized_size(section: &LegalizedSection, placed: &PlacedSection) -> Result<u32, RelaxError> {
    let mut cursor = 0_u32;
    let mut items = Vec::new();
    items.extend(section.labels.iter().map(|item| (item.seq_index, 0_u32)));
    items.extend(
        section
            .instrs
            .iter()
            .map(|item| (item.seq_index, u32::from(item.data.byte_len()))),
    );
    items.extend(section.data_blocks.iter().map(|item| {
        (
            item.seq_index,
            match &item.data {
                DataBlock::Bytes(bytes) => bytes.len() as u32,
                DataBlock::Words(words) => words.len() as u32 * 2,
            },
        )
    }));
    for item in &section.alignments {
        let padding = placed.alignment_padding.get(&item.seq_index).ok_or(
            RelaxError::MissingAlignmentPlan {
                section_id: section.id,
                seq_index: item.seq_index,
            },
        )?;
        items.push((item.seq_index, u32::from(*padding)));
    }
    items.sort_by_key(|(seq, _)| *seq);
    for (_, size) in items {
        cursor += size;
    }
    Ok(cursor)
}

#[derive(Debug, Clone, Copy)]
enum OffsetItem {
    Label,
    Fixed(u32),
    Align(Option<u16>),
}

fn materialize_stub_thunk(
    id: SectionId,
    _cpu_start: u16,
    request: &ResolvedThunkRequest,
) -> LegalizedSection {
    let provenance = InstrProvenance::new(PlanningStage::Backend).with_source_op("stub_thunk");
    let bank = match request.callee_bank {
        BankIndex::Rom(bank) => bank,
        _ => 0,
    };
    LegalizedSection {
        id,
        role: SectionRole::Bank0Nucleus,
        name: request.thunk_symbol.clone(),
        align: NonZeroU16::new(1).expect("nonzero"),
        size_hint_bytes: Some(10),
        next_seq_index: 5,
        labels: vec![OrderedItem::new(
            Label {
                id: crate::section::SymbolId::new(0),
                name: request.thunk_symbol.clone(),
            },
            0,
            provenance.clone(),
        )],
        instrs: vec![
            OrderedItem::new(
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm: bank as u8,
                },
                1,
                provenance.clone(),
            ),
            OrderedItem::new(
                Instr::Ld8RegFromImm {
                    dst: Reg8::B,
                    imm: (bank >> 8) as u8,
                },
                2,
                provenance.clone(),
            ),
            OrderedItem::new(
                Instr::Ld16Imm {
                    dst: Reg16Data::HL,
                    imm: request.target_cpu_addr,
                },
                3,
                provenance.clone(),
            ),
            OrderedItem::new(
                Instr::JpAbs {
                    cond: None,
                    addr: 0x0150,
                },
                4,
                provenance,
            ),
        ],
        data_blocks: vec![],
        alignments: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{PinnedPlacement, PlacementProfile, layout_into_banks};
    use crate::section::{SymbolId, SymbolicBranch};

    fn prov() -> InstrProvenance {
        InstrProvenance::new(PlanningStage::Backend)
    }

    fn empty_section(id: u32, role: SectionRole, name: SymbolName) -> LoweredSection {
        LoweredSection {
            id: SectionId::new(id),
            role,
            name,
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 0,
            labels: vec![],
            instrs: vec![],
            data_blocks: vec![],
            alignments: vec![],
            legalization_ops: vec![],
            branches: vec![],
        }
    }

    #[test]
    fn out_of_range_jr_becomes_jp() {
        let target = SymbolName::runtime("test", "far_label").expect("symbol");
        let mut section = empty_section(
            1,
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        section.branches.push(OrderedItem::new(
            SymbolicBranch::jump(target.clone(), None),
            0,
            prov(),
        ));
        for idx in 1..=200 {
            section
                .instrs
                .push(OrderedItem::new(Instr::Nop, idx, prov()));
        }
        section.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target,
            },
            201,
            prov(),
        ));

        let layout = layout_into_banks(&[section.clone()], PlacementProfile::PackedExperts, &[])
            .expect("layout succeeds");
        let relaxed = relax_and_legalize(&[section], &layout).expect("relax succeeds");
        assert!(matches!(
            relaxed.sections[0].instrs[0].data,
            Instr::JpAbs { cond: None, .. }
        ));
    }

    #[test]
    fn same_bank_jr_stays_short() {
        let target = SymbolName::runtime("test", "near_label").expect("symbol");
        let mut section = empty_section(
            1,
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        section.branches.push(OrderedItem::new(
            SymbolicBranch::jump(target.clone(), None),
            0,
            prov(),
        ));
        section.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target,
            },
            1,
            prov(),
        ));
        let layout = layout_into_banks(&[section.clone()], PlacementProfile::PackedExperts, &[])
            .expect("layout succeeds");
        let relaxed = relax_and_legalize(&[section], &layout).expect("relax succeeds");
        assert_eq!(
            relaxed.sections[0].instrs[0].data,
            Instr::JrRel { cond: None, off: 0 }
        );
    }

    #[test]
    fn cross_bank_jr_is_rejected() {
        let target = SymbolName::runtime("test", "target").expect("symbol");
        let mut caller = empty_section(
            1,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        caller.branches.push(OrderedItem::new(
            SymbolicBranch::jump(target.clone(), None),
            0,
            prov(),
        ));
        let mut callee = empty_section(
            2,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "callee").expect("symbol"),
        );
        callee.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target.clone(),
            },
            0,
            prov(),
        ));
        let sections = vec![caller, callee];
        let layout = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[
                PinnedPlacement {
                    section_id: SectionId::new(1),
                    bank: BankIndex::Rom(1),
                    cpu_start: 0x4000,
                },
                PinnedPlacement {
                    section_id: SectionId::new(2),
                    bank: BankIndex::Rom(2),
                    cpu_start: 0x4000,
                },
            ],
        )
        .expect("layout succeeds");
        let err = relax_and_legalize(&sections, &layout).expect_err("cross-bank jr rejected");
        assert!(matches!(
            err,
            RelaxError::CrossBankBranchUnsupported {
                source_bank: BankIndex::Rom(1),
                target_bank: BankIndex::Rom(2),
                ..
            }
        ));
    }

    #[test]
    fn plain_cross_bank_call_is_rejected() {
        let target = SymbolName::runtime("test", "target").expect("symbol");
        let mut caller = empty_section(
            1,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        caller.branches.push(OrderedItem::new(
            SymbolicBranch::call(target.clone(), None),
            0,
            prov(),
        ));
        let mut callee = empty_section(
            2,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "callee").expect("symbol"),
        );
        callee.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target,
            },
            0,
            prov(),
        ));
        let sections = vec![caller, callee];
        let layout = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[
                PinnedPlacement {
                    section_id: SectionId::new(1),
                    bank: BankIndex::Rom(1),
                    cpu_start: 0x4000,
                },
                PinnedPlacement {
                    section_id: SectionId::new(2),
                    bank: BankIndex::Rom(2),
                    cpu_start: 0x4000,
                },
            ],
        )
        .expect("layout succeeds");
        assert!(matches!(
            relax_and_legalize(&sections, &layout),
            Err(RelaxError::CrossBankBranchUnsupported { .. })
        ));
    }

    #[test]
    fn explicit_far_call_becomes_per_target_thunk() {
        let target = SymbolName::runtime("test", "target").expect("symbol");
        let mut caller = empty_section(
            1,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        caller.legalization_ops.push(OrderedItem::new(
            LegalizationOp::FarCall {
                target: target.clone(),
                lease_chain: vec![],
            },
            0,
            prov(),
        ));
        let mut callee = empty_section(
            2,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "callee").expect("symbol"),
        );
        callee.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target.clone(),
            },
            0,
            prov(),
        ));
        let sections = vec![caller, callee];
        let layout = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[
                PinnedPlacement {
                    section_id: SectionId::new(1),
                    bank: BankIndex::Rom(1),
                    cpu_start: 0x4000,
                },
                PinnedPlacement {
                    section_id: SectionId::new(2),
                    bank: BankIndex::Rom(2),
                    cpu_start: 0x4000,
                },
            ],
        )
        .expect("layout succeeds");
        let relaxed = relax_and_legalize(&sections, &layout).expect("relax succeeds");
        assert_eq!(relaxed.thunk_requests.len(), 1);
        assert_eq!(relaxed.thunk_requests[0].target, target);
        assert_eq!(
            relaxed.sections[0].instrs[0].data,
            Instr::Call {
                cond: None,
                addr: 0x3F00,
            }
        );
        assert!(
            relaxed
                .sections
                .iter()
                .any(|section| section.name.as_str().starts_with("runtime.banking.thunk."))
        );
    }

    #[test]
    fn two_callsites_share_one_thunk() {
        let target = SymbolName::runtime("test", "target").expect("symbol");
        let mut caller = empty_section(
            1,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "caller").expect("symbol"),
        );
        caller.legalization_ops.push(OrderedItem::new(
            LegalizationOp::FarCall {
                target: target.clone(),
                lease_chain: vec![],
            },
            0,
            prov(),
        ));
        caller.legalization_ops.push(OrderedItem::new(
            LegalizationOp::FarCall {
                target: target.clone(),
                lease_chain: vec![],
            },
            1,
            prov(),
        ));
        let mut callee = empty_section(
            2,
            SectionRole::CommonBank,
            SymbolName::runtime("test", "callee").expect("symbol"),
        );
        callee.labels.push(OrderedItem::new(
            Label {
                id: SymbolId::new(0),
                name: target,
            },
            0,
            prov(),
        ));
        let sections = vec![caller, callee];
        let layout = layout_into_banks(
            &sections,
            PlacementProfile::PackedExperts,
            &[
                PinnedPlacement {
                    section_id: SectionId::new(1),
                    bank: BankIndex::Rom(1),
                    cpu_start: 0x4000,
                },
                PinnedPlacement {
                    section_id: SectionId::new(2),
                    bank: BankIndex::Rom(2),
                    cpu_start: 0x4000,
                },
            ],
        )
        .expect("layout succeeds");
        let relaxed = relax_and_legalize(&sections, &layout).expect("relax succeeds");
        assert_eq!(relaxed.thunk_requests.len(), 1);
        assert_eq!(relaxed.sections[0].instrs.len(), 2);
        assert_eq!(
            relaxed.sections[0].instrs[0].data,
            relaxed.sections[0].instrs[1].data
        );
    }
}
