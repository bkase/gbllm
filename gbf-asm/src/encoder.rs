//! Canonical LR35902 instruction and section encoder.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::isa::{
    AluSrc8, CbTarget, Cond, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data, Reg16Stack,
    RstVector,
};
use crate::layout::{AddressSpace, LayoutError, PlacedSection};
use crate::section::{DataBlock, LegalizedSection, SectionId};

/// Byte used for layout-selected alignment padding in ROM sections.
///
/// `0xFF` matches erased/fill ROM bytes and avoids silently creating executable
/// `NOP` padding.
pub const PAD_BYTE: u8 = 0xFF;

/// One section after byte lowering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedSection {
    pub id: SectionId,
    pub bytes: Vec<u8>,
    pub item_spans: Vec<EncodedItemSpan>,
}

/// Byte span for one ordered section item that materialized bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EncodedItemSpan {
    pub seq_index: u32,
    pub kind: EncodedItemKind,
    pub offset: u16,
    pub len: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodedItemKind {
    Instr,
    DataBlock,
    AlignmentPadding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    EncodedLengthMismatch {
        expected: u8,
        actual: u8,
        instr: Instr,
    },
    MissingAlignmentPlan {
        section_id: SectionId,
        seq_index: u32,
    },
    ExtraAlignmentPlan {
        section_id: SectionId,
        seq_index: u32,
    },
    NonRomSectionEncoded {
        section_id: SectionId,
        space: AddressSpace,
    },
    SectionPlacementMismatch {
        section_id: SectionId,
        placed_id: SectionId,
    },
    InvalidPlacement {
        section_id: SectionId,
        error: LayoutError,
    },
    SectionSizeMismatch {
        section_id: SectionId,
        expected: u16,
        actual: usize,
    },
    SectionOffsetOverflow {
        section_id: SectionId,
        offset: usize,
    },
    ItemSpanOverflow {
        section_id: SectionId,
        seq_index: u32,
        len: usize,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncodedLengthMismatch {
                expected,
                actual,
                instr,
            } => write!(
                f,
                "encoded {instr:?} produced {actual} bytes, expected {expected}"
            ),
            Self::MissingAlignmentPlan {
                section_id,
                seq_index,
            } => write!(
                f,
                "section {} has Align item {seq_index} but no layout padding entry",
                section_id.get()
            ),
            Self::ExtraAlignmentPlan {
                section_id,
                seq_index,
            } => write!(
                f,
                "section {} layout padding references non-Align item {seq_index}",
                section_id.get()
            ),
            Self::NonRomSectionEncoded { section_id, space } => write!(
                f,
                "section {} has non-ROM address space {space:?} and cannot be encoded",
                section_id.get()
            ),
            Self::SectionPlacementMismatch {
                section_id,
                placed_id,
            } => write!(
                f,
                "section {} was paired with placement for section {}",
                section_id.get(),
                placed_id.get()
            ),
            Self::InvalidPlacement { section_id, error } => {
                write!(
                    f,
                    "section {} has invalid placement: {error}",
                    section_id.get()
                )
            }
            Self::SectionSizeMismatch {
                section_id,
                expected,
                actual,
            } => write!(
                f,
                "section {} encoded to {actual} bytes, layout expected {expected}",
                section_id.get()
            ),
            Self::SectionOffsetOverflow { section_id, offset } => write!(
                f,
                "section {} item offset {offset} does not fit in u16",
                section_id.get()
            ),
            Self::ItemSpanOverflow {
                section_id,
                seq_index,
                len,
            } => write!(
                f,
                "section {} item {seq_index} span length {len} does not fit in u16",
                section_id.get()
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encode one fully-lowered, fully-relaxed section.
pub fn encode_section(
    section: &LegalizedSection,
    placed: &PlacedSection,
) -> Result<EncodedSection, EncodeError> {
    if section.id != placed.id {
        return Err(EncodeError::SectionPlacementMismatch {
            section_id: section.id,
            placed_id: placed.id,
        });
    }
    match placed.rom_file_offset() {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Err(EncodeError::NonRomSectionEncoded {
                section_id: section.id,
                space: placed.space,
            });
        }
        Err(error) => {
            return Err(EncodeError::InvalidPlacement {
                section_id: section.id,
                error,
            });
        }
    }

    let mut items = Vec::with_capacity(
        section.labels.len()
            + section.instrs.len()
            + section.data_blocks.len()
            + section.alignments.len(),
    );
    items.extend(
        section
            .labels
            .iter()
            .map(|item| (item.seq_index, SectionEncodeItem::Label)),
    );
    items.extend(
        section
            .instrs
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.seq_index, SectionEncodeItem::Instr(idx))),
    );
    items.extend(
        section
            .data_blocks
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.seq_index, SectionEncodeItem::DataBlock(idx))),
    );
    items.extend(
        section
            .alignments
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.seq_index, SectionEncodeItem::Align(idx))),
    );
    items.sort_by_key(|(seq_index, _)| *seq_index);

    let mut bytes = Vec::with_capacity(usize::from(placed.final_size));
    let mut item_spans = Vec::new();
    let mut seen_alignments = std::collections::BTreeSet::new();

    for (seq_index, item) in items {
        match item {
            SectionEncodeItem::Label => {}
            SectionEncodeItem::Instr(idx) => {
                let instr = &section.instrs[idx].data;
                let encoded = encode_instr(instr)?;
                let offset = checked_offset(bytes.len(), section.id)?;
                bytes.extend_from_slice(&encoded);
                item_spans.push(EncodedItemSpan {
                    seq_index,
                    kind: EncodedItemKind::Instr,
                    offset,
                    len: checked_span_len(encoded.len(), section.id, seq_index)?,
                });
            }
            SectionEncodeItem::DataBlock(idx) => {
                let offset = checked_offset(bytes.len(), section.id)?;
                let before = bytes.len();
                encode_data_block(&section.data_blocks[idx].data, &mut bytes);
                item_spans.push(EncodedItemSpan {
                    seq_index,
                    kind: EncodedItemKind::DataBlock,
                    offset,
                    len: checked_span_len(bytes.len() - before, section.id, seq_index)?,
                });
            }
            SectionEncodeItem::Align(idx) => {
                let align = &section.alignments[idx];
                seen_alignments.insert(align.seq_index);
                let padding = *placed.alignment_padding.get(&align.seq_index).ok_or(
                    EncodeError::MissingAlignmentPlan {
                        section_id: section.id,
                        seq_index: align.seq_index,
                    },
                )?;
                let offset = checked_offset(bytes.len(), section.id)?;
                bytes.resize(bytes.len() + usize::from(padding), PAD_BYTE);
                item_spans.push(EncodedItemSpan {
                    seq_index,
                    kind: EncodedItemKind::AlignmentPadding,
                    offset,
                    len: padding,
                });
            }
        }
    }

    for seq_index in placed.alignment_padding.keys() {
        if !seen_alignments.contains(seq_index) {
            return Err(EncodeError::ExtraAlignmentPlan {
                section_id: section.id,
                seq_index: *seq_index,
            });
        }
    }

    if bytes.len() != usize::from(placed.final_size) {
        return Err(EncodeError::SectionSizeMismatch {
            section_id: section.id,
            expected: placed.final_size,
            actual: bytes.len(),
        });
    }

    Ok(EncodedSection {
        id: section.id,
        bytes,
        item_spans,
    })
}

#[derive(Debug, Clone, Copy)]
enum SectionEncodeItem {
    Label,
    Instr(usize),
    DataBlock(usize),
    Align(usize),
}

fn checked_offset(offset: usize, section_id: SectionId) -> Result<u16, EncodeError> {
    u16::try_from(offset).map_err(|_| EncodeError::SectionOffsetOverflow { section_id, offset })
}

fn checked_span_len(len: usize, section_id: SectionId, seq_index: u32) -> Result<u16, EncodeError> {
    u16::try_from(len).map_err(|_| EncodeError::ItemSpanOverflow {
        section_id,
        seq_index,
        len,
    })
}

fn encode_data_block(block: &DataBlock, out: &mut Vec<u8>) {
    match block {
        DataBlock::Bytes(bytes) => out.extend_from_slice(bytes),
        DataBlock::Words(words) => {
            for word in words {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
    }
}

/// Encode one concrete LR35902 instruction.
pub fn encode_instr(instr: &Instr) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(usize::from(instr.byte_len()));
    match *instr {
        Instr::Nop => out.push(0x00),
        Instr::Stop => out.extend_from_slice(&[0x10, 0x00]),
        Instr::Halt => out.push(0x76),
        Instr::Di => out.push(0xF3),
        Instr::Ei => out.push(0xFB),
        Instr::Ccf => out.push(0x3F),
        Instr::Scf => out.push(0x37),
        Instr::Cpl => out.push(0x2F),
        Instr::Daa => out.push(0x27),
        Instr::Ld8Reg { dst, src } => out.push(0x40 | (reg8_code(dst) << 3) | reg8_code(src)),
        Instr::Ld8RegFromImm { dst, imm } => {
            out.extend_from_slice(&[0x06 | (reg8_code(dst) << 3), imm])
        }
        Instr::Ld8RegFromHl { dst } => out.push(0x46 | (reg8_code(dst) << 3)),
        Instr::Ld8HlFromReg { src } => out.push(0x70 | reg8_code(src)),
        Instr::Ld8HlFromImm { imm } => out.extend_from_slice(&[0x36, imm]),
        Instr::LdAFromReg16Addr { src } => out.push(match src {
            Reg16Addr::BC => 0x0A,
            Reg16Addr::DE => 0x1A,
            Reg16Addr::Hli => 0x2A,
            Reg16Addr::Hld => 0x3A,
        }),
        Instr::LdReg16AddrFromA { dst } => out.push(match dst {
            Reg16Addr::BC => 0x02,
            Reg16Addr::DE => 0x12,
            Reg16Addr::Hli => 0x22,
            Reg16Addr::Hld => 0x32,
        }),
        Instr::LdAFromDirect { addr } => push_u16(&mut out, 0xFA, addr.get()),
        Instr::LdDirectFromA { addr } => push_u16(&mut out, 0xEA, addr.get()),
        Instr::LdAFromHighDirect { offset } => out.extend_from_slice(&[0xF0, offset.get()]),
        Instr::LdHighDirectFromA { offset } => out.extend_from_slice(&[0xE0, offset.get()]),
        Instr::LdAFromHighC => out.push(0xF2),
        Instr::LdHighCFromA => out.push(0xE2),
        Instr::Ld16Imm { dst, imm } => push_u16(&mut out, 0x01 | (reg16_data_code(dst) << 4), imm),
        Instr::LdSpFromHl => out.push(0xF9),
        Instr::LdDirectFromSp { addr } => push_u16(&mut out, 0x08, addr),
        Instr::LdHlFromSpPlus { off } => out.extend_from_slice(&[0xF8, off as u8]),
        Instr::AddA { src } => encode_alu(&mut out, 0x80, 0xC6, src),
        Instr::AdcA { src } => encode_alu(&mut out, 0x88, 0xCE, src),
        Instr::SubA { src } => encode_alu(&mut out, 0x90, 0xD6, src),
        Instr::SbcA { src } => encode_alu(&mut out, 0x98, 0xDE, src),
        Instr::AndA { src } => encode_alu(&mut out, 0xA0, 0xE6, src),
        Instr::XorA { src } => encode_alu(&mut out, 0xA8, 0xEE, src),
        Instr::OrA { src } => encode_alu(&mut out, 0xB0, 0xF6, src),
        Instr::CpA { src } => encode_alu(&mut out, 0xB8, 0xFE, src),
        Instr::Inc8 { dst } => out.push(match dst {
            IncDec8Target::Reg(reg) => 0x04 | (reg8_code(reg) << 3),
            IncDec8Target::HlIndirect => 0x34,
        }),
        Instr::Dec8 { dst } => out.push(match dst {
            IncDec8Target::Reg(reg) => 0x05 | (reg8_code(reg) << 3),
            IncDec8Target::HlIndirect => 0x35,
        }),
        Instr::Inc16 { dst } => out.push(0x03 | (reg16_data_code(dst) << 4)),
        Instr::Dec16 { dst } => out.push(0x0B | (reg16_data_code(dst) << 4)),
        Instr::AddHl { src } => out.push(0x09 | (reg16_data_code(src) << 4)),
        Instr::AddSp { off } => out.extend_from_slice(&[0xE8, off as u8]),
        Instr::Rlca => out.push(0x07),
        Instr::Rrca => out.push(0x0F),
        Instr::Rla => out.push(0x17),
        Instr::Rra => out.push(0x1F),
        Instr::Rlc { target } => encode_cb(&mut out, 0x00, target),
        Instr::Rrc { target } => encode_cb(&mut out, 0x08, target),
        Instr::Rl { target } => encode_cb(&mut out, 0x10, target),
        Instr::Rr { target } => encode_cb(&mut out, 0x18, target),
        Instr::Sla { target } => encode_cb(&mut out, 0x20, target),
        Instr::Sra { target } => encode_cb(&mut out, 0x28, target),
        Instr::Swap { target } => encode_cb(&mut out, 0x30, target),
        Instr::Srl { target } => encode_cb(&mut out, 0x38, target),
        Instr::Bit { bit, target } => encode_cb(&mut out, 0x40 | (bit.get() << 3), target),
        Instr::Res { bit, target } => encode_cb(&mut out, 0x80 | (bit.get() << 3), target),
        Instr::Set { bit, target } => encode_cb(&mut out, 0xC0 | (bit.get() << 3), target),
        Instr::JpAbs { cond, addr } => push_u16(&mut out, jp_opcode(cond), addr),
        Instr::JpHl => out.push(0xE9),
        Instr::JrRel { cond, off } => out.extend_from_slice(&[jr_opcode(cond), off as u8]),
        Instr::Call { cond, addr } => push_u16(&mut out, call_opcode(cond), addr),
        Instr::Ret { cond } => out.push(ret_opcode(cond)),
        Instr::Reti => out.push(0xD9),
        Instr::Rst { vector } => out.push(rst_opcode(vector)),
        Instr::Push { src } => out.push(0xC5 | (reg16_stack_code(src) << 4)),
        Instr::Pop { dst } => out.push(0xC1 | (reg16_stack_code(dst) << 4)),
    }

    if out.len() != usize::from(instr.byte_len()) {
        return Err(EncodeError::EncodedLengthMismatch {
            expected: instr.byte_len(),
            actual: out.len() as u8,
            instr: *instr,
        });
    }

    Ok(out)
}

fn push_u16(out: &mut Vec<u8>, opcode: u8, value: u16) {
    out.push(opcode);
    out.extend_from_slice(&value.to_le_bytes());
}

fn encode_alu(out: &mut Vec<u8>, base: u8, imm_opcode: u8, src: AluSrc8) {
    match src {
        AluSrc8::Reg(reg) => out.push(base | reg8_code(reg)),
        AluSrc8::HlIndirect => out.push(base | 0x06),
        AluSrc8::Imm(imm) => out.extend_from_slice(&[imm_opcode, imm]),
    }
}

fn encode_cb(out: &mut Vec<u8>, base: u8, target: CbTarget) {
    out.extend_from_slice(&[0xCB, base | cb_target_code(target)]);
}

fn reg8_code(reg: Reg8) -> u8 {
    match reg {
        Reg8::B => 0,
        Reg8::C => 1,
        Reg8::D => 2,
        Reg8::E => 3,
        Reg8::H => 4,
        Reg8::L => 5,
        Reg8::A => 7,
    }
}

fn cb_target_code(target: CbTarget) -> u8 {
    match target {
        CbTarget::Reg(reg) => reg8_code(reg),
        CbTarget::HlIndirect => 6,
    }
}

fn reg16_data_code(reg: Reg16Data) -> u8 {
    match reg {
        Reg16Data::BC => 0,
        Reg16Data::DE => 1,
        Reg16Data::HL => 2,
        Reg16Data::SP => 3,
    }
}

fn reg16_stack_code(reg: Reg16Stack) -> u8 {
    match reg {
        Reg16Stack::BC => 0,
        Reg16Stack::DE => 1,
        Reg16Stack::HL => 2,
        Reg16Stack::AF => 3,
    }
}

fn jp_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xC3,
        Some(Cond::NZ) => 0xC2,
        Some(Cond::Z) => 0xCA,
        Some(Cond::NC) => 0xD2,
        Some(Cond::C) => 0xDA,
    }
}

fn jr_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0x18,
        Some(Cond::NZ) => 0x20,
        Some(Cond::Z) => 0x28,
        Some(Cond::NC) => 0x30,
        Some(Cond::C) => 0x38,
    }
}

fn call_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xCD,
        Some(Cond::NZ) => 0xC4,
        Some(Cond::Z) => 0xCC,
        Some(Cond::NC) => 0xD4,
        Some(Cond::C) => 0xDC,
    }
}

fn ret_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xC9,
        Some(Cond::NZ) => 0xC0,
        Some(Cond::Z) => 0xC8,
        Some(Cond::NC) => 0xD0,
        Some(Cond::C) => 0xD8,
    }
}

fn rst_opcode(vector: RstVector) -> u8 {
    0xC7 | vector.addr()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU16;

    use super::*;
    use crate::cycle_model::sample_instrs;
    use crate::isa::{
        AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Instr,
        Reg8, Reg16Addr, Reg16Data, Reg16Stack, RstVector,
    };
    use crate::layout::{AddressSpace, BankIndex};
    use crate::provenance::{InstrProvenance, PlanningStage};
    use crate::section::{
        Align, Label, LegalizedSection, OrderedItem, SectionId, SectionRole, SymbolId,
    };
    use crate::symbols::SymbolName;

    fn enc(instr: Instr) -> Vec<u8> {
        encode_instr(&instr).expect("instruction encodes")
    }

    fn direct(addr: u16) -> DirectAddr {
        DirectAddr::new(addr).expect("valid direct address")
    }

    #[test]
    fn known_opcodes() {
        let cases: &[(Instr, &[u8])] = &[
            (Instr::Nop, &[0x00]),
            (Instr::Stop, &[0x10, 0x00]),
            (Instr::Halt, &[0x76]),
            (Instr::Di, &[0xF3]),
            (Instr::Ei, &[0xFB]),
            (Instr::Ccf, &[0x3F]),
            (Instr::Scf, &[0x37]),
            (Instr::Cpl, &[0x2F]),
            (Instr::Daa, &[0x27]),
            (
                Instr::Ld8Reg {
                    dst: Reg8::A,
                    src: Reg8::B,
                },
                &[0x78],
            ),
            (
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm: 0x42,
                },
                &[0x3E, 0x42],
            ),
            (Instr::Ld8RegFromHl { dst: Reg8::A }, &[0x7E]),
            (Instr::Ld8HlFromReg { src: Reg8::A }, &[0x77]),
            (Instr::Ld8HlFromImm { imm: 0x12 }, &[0x36, 0x12]),
            (
                Instr::LdAFromReg16Addr {
                    src: Reg16Addr::Hli,
                },
                &[0x2A],
            ),
            (
                Instr::LdReg16AddrFromA {
                    dst: Reg16Addr::Hld,
                },
                &[0x32],
            ),
            (
                Instr::LdAFromDirect {
                    addr: direct(0xC123),
                },
                &[0xFA, 0x23, 0xC1],
            ),
            (
                Instr::LdDirectFromA {
                    addr: direct(0xC123),
                },
                &[0xEA, 0x23, 0xC1],
            ),
            (
                Instr::LdAFromHighDirect {
                    offset: HighDirectOffset::new(0x44),
                },
                &[0xF0, 0x44],
            ),
            (
                Instr::LdHighDirectFromA {
                    offset: HighDirectOffset::new(0x44),
                },
                &[0xE0, 0x44],
            ),
            (Instr::LdAFromHighC, &[0xF2]),
            (Instr::LdHighCFromA, &[0xE2]),
            (
                Instr::Ld16Imm {
                    dst: Reg16Data::HL,
                    imm: 0xCAFE,
                },
                &[0x21, 0xFE, 0xCA],
            ),
            (Instr::LdSpFromHl, &[0xF9]),
            (Instr::LdDirectFromSp { addr: 0xC000 }, &[0x08, 0x00, 0xC0]),
            (Instr::LdHlFromSpPlus { off: -4 }, &[0xF8, 0xFC]),
            (
                Instr::AddA {
                    src: AluSrc8::Reg(Reg8::B),
                },
                &[0x80],
            ),
            (
                Instr::AddA {
                    src: AluSrc8::Imm(0x12),
                },
                &[0xC6, 0x12],
            ),
            (
                Instr::AdcA {
                    src: AluSrc8::HlIndirect,
                },
                &[0x8E],
            ),
            (
                Instr::SubA {
                    src: AluSrc8::Reg(Reg8::C),
                },
                &[0x91],
            ),
            (
                Instr::SbcA {
                    src: AluSrc8::Imm(0x34),
                },
                &[0xDE, 0x34],
            ),
            (
                Instr::AndA {
                    src: AluSrc8::Reg(Reg8::D),
                },
                &[0xA2],
            ),
            (
                Instr::XorA {
                    src: AluSrc8::HlIndirect,
                },
                &[0xAE],
            ),
            (
                Instr::OrA {
                    src: AluSrc8::Imm(0x56),
                },
                &[0xF6, 0x56],
            ),
            (
                Instr::CpA {
                    src: AluSrc8::Reg(Reg8::E),
                },
                &[0xBB],
            ),
            (
                Instr::Inc8 {
                    dst: IncDec8Target::Reg(Reg8::A),
                },
                &[0x3C],
            ),
            (
                Instr::Dec8 {
                    dst: IncDec8Target::HlIndirect,
                },
                &[0x35],
            ),
            (Instr::Inc16 { dst: Reg16Data::SP }, &[0x33]),
            (Instr::Dec16 { dst: Reg16Data::DE }, &[0x1B]),
            (Instr::AddHl { src: Reg16Data::BC }, &[0x09]),
            (Instr::AddSp { off: -2 }, &[0xE8, 0xFE]),
            (Instr::Rlca, &[0x07]),
            (Instr::Rrca, &[0x0F]),
            (Instr::Rla, &[0x17]),
            (Instr::Rra, &[0x1F]),
            (
                Instr::Bit {
                    bit: BitIndex::B7,
                    target: CbTarget::Reg(Reg8::H),
                },
                &[0xCB, 0x7C],
            ),
            (
                Instr::Swap {
                    target: CbTarget::HlIndirect,
                },
                &[0xCB, 0x36],
            ),
            (
                Instr::JpAbs {
                    cond: None,
                    addr: 0x0150,
                },
                &[0xC3, 0x50, 0x01],
            ),
            (
                Instr::JpAbs {
                    cond: Some(Cond::NZ),
                    addr: 0x4000,
                },
                &[0xC2, 0x00, 0x40],
            ),
            (Instr::JpHl, &[0xE9]),
            (
                Instr::JrRel {
                    cond: None,
                    off: -2,
                },
                &[0x18, 0xFE],
            ),
            (
                Instr::JrRel {
                    cond: Some(Cond::C),
                    off: 0x7F,
                },
                &[0x38, 0x7F],
            ),
            (
                Instr::Call {
                    cond: None,
                    addr: 0x4000,
                },
                &[0xCD, 0x00, 0x40],
            ),
            (
                Instr::Call {
                    cond: Some(Cond::NC),
                    addr: 0x1234,
                },
                &[0xD4, 0x34, 0x12],
            ),
            (Instr::Ret { cond: None }, &[0xC9]),
            (
                Instr::Ret {
                    cond: Some(Cond::Z),
                },
                &[0xC8],
            ),
            (Instr::Reti, &[0xD9]),
            (
                Instr::Rst {
                    vector: RstVector::V38,
                },
                &[0xFF],
            ),
            (
                Instr::Push {
                    src: Reg16Stack::AF,
                },
                &[0xF5],
            ),
            (
                Instr::Pop {
                    dst: Reg16Stack::HL,
                },
                &[0xE1],
            ),
        ];

        for (instr, expected) in cases {
            assert_eq!(enc(*instr), *expected, "{instr:?}");
        }
    }

    #[test]
    fn cb_prefix_table_is_exhaustive() {
        let targets = [
            CbTarget::Reg(Reg8::B),
            CbTarget::Reg(Reg8::C),
            CbTarget::Reg(Reg8::D),
            CbTarget::Reg(Reg8::E),
            CbTarget::Reg(Reg8::H),
            CbTarget::Reg(Reg8::L),
            CbTarget::HlIndirect,
            CbTarget::Reg(Reg8::A),
        ];
        for (code, target) in targets.into_iter().enumerate() {
            assert_eq!(enc(Instr::Rlc { target }), vec![0xCB, code as u8]);
            assert_eq!(enc(Instr::Rrc { target }), vec![0xCB, 0x08 | code as u8]);
            assert_eq!(enc(Instr::Rl { target }), vec![0xCB, 0x10 | code as u8]);
            assert_eq!(enc(Instr::Rr { target }), vec![0xCB, 0x18 | code as u8]);
            assert_eq!(enc(Instr::Sla { target }), vec![0xCB, 0x20 | code as u8]);
            assert_eq!(enc(Instr::Sra { target }), vec![0xCB, 0x28 | code as u8]);
            assert_eq!(enc(Instr::Swap { target }), vec![0xCB, 0x30 | code as u8]);
            assert_eq!(enc(Instr::Srl { target }), vec![0xCB, 0x38 | code as u8]);
            for bit in 0..8 {
                let bit = BitIndex::new(bit).expect("valid bit");
                assert_eq!(
                    enc(Instr::Bit { bit, target }),
                    vec![0xCB, 0x40 | (bit.get() << 3) | code as u8]
                );
                assert_eq!(
                    enc(Instr::Res { bit, target }),
                    vec![0xCB, 0x80 | (bit.get() << 3) | code as u8]
                );
                assert_eq!(
                    enc(Instr::Set { bit, target }),
                    vec![0xCB, 0xC0 | (bit.get() << 3) | code as u8]
                );
            }
        }
    }

    #[test]
    fn encode_instr_matches_byte_len() {
        for instr in sample_instrs() {
            assert_eq!(enc(instr).len(), instr.byte_len() as usize, "{instr:?}");
        }
    }

    #[test]
    fn encode_section_merges_legalized_arrays_in_order() {
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let section = LegalizedSection {
            id: SectionId::new(9),
            role: SectionRole::HeaderCartridge,
            name: SymbolName::runtime("test", "section").expect("symbol"),
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 5,
            labels: vec![OrderedItem::new(
                Label {
                    id: SymbolId::new(0),
                    name: SymbolName::runtime("test", "entry").expect("symbol"),
                },
                0,
                prov.clone(),
            )],
            instrs: vec![OrderedItem::new(Instr::Nop, 1, prov.clone())],
            data_blocks: vec![OrderedItem::new(
                DataBlock::Words(vec![0x1234]),
                3,
                prov.clone(),
            )],
            alignments: vec![OrderedItem::new(
                Align(NonZeroU16::new(4).expect("nonzero")),
                2,
                prov,
            )],
        };
        let mut alignment_padding = BTreeMap::new();
        alignment_padding.insert(2, 3);
        let placed = PlacedSection {
            id: section.id,
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start: 0x0150,
            final_size: 6,
            estimated_size: 6,
            alignment_padding,
        };

        let encoded = encode_section(&section, &placed).expect("section encodes");
        assert_eq!(encoded.bytes, vec![0x00, 0xFF, 0xFF, 0xFF, 0x34, 0x12]);
        assert_eq!(
            encoded.item_spans,
            vec![
                EncodedItemSpan {
                    seq_index: 1,
                    kind: EncodedItemKind::Instr,
                    offset: 0,
                    len: 1,
                },
                EncodedItemSpan {
                    seq_index: 2,
                    kind: EncodedItemKind::AlignmentPadding,
                    offset: 1,
                    len: 3,
                },
                EncodedItemSpan {
                    seq_index: 3,
                    kind: EncodedItemKind::DataBlock,
                    offset: 4,
                    len: 2,
                },
            ]
        );
    }

    #[test]
    fn encode_section_rejects_mismatched_or_invalid_placement() {
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let section = LegalizedSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Nucleus,
            name: SymbolName::runtime("test", "section").expect("symbol"),
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 1,
            labels: vec![],
            instrs: vec![OrderedItem::new(Instr::Nop, 0, prov)],
            data_blocks: vec![],
            alignments: vec![],
        };
        let mut placed = PlacedSection {
            id: SectionId::new(2),
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start: 0x0150,
            final_size: 1,
            estimated_size: 1,
            alignment_padding: BTreeMap::new(),
        };

        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::SectionPlacementMismatch {
                section_id,
                placed_id,
            }) if section_id == SectionId::new(1) && placed_id == SectionId::new(2)
        ));

        placed.id = section.id;
        placed.space = AddressSpace::Rom0;
        placed.bank = BankIndex::Rom(1);
        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::InvalidPlacement { .. })
        ));

        placed.space = AddressSpace::Rom0;
        placed.bank = BankIndex::Rom(0);
        placed.cpu_start = 0x3FFF;
        placed.final_size = 2;
        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::InvalidPlacement { .. })
        ));
    }

    #[test]
    fn encode_section_rejects_missing_or_extra_alignment_plan() {
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let section = LegalizedSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Nucleus,
            name: SymbolName::runtime("test", "alignment").expect("symbol"),
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 2,
            labels: vec![],
            instrs: vec![],
            data_blocks: vec![],
            alignments: vec![OrderedItem::new(
                Align(NonZeroU16::new(4).expect("nonzero")),
                0,
                prov,
            )],
        };
        let mut placed = PlacedSection {
            id: section.id,
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start: 0x0150,
            final_size: 0,
            estimated_size: 0,
            alignment_padding: BTreeMap::new(),
        };
        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::MissingAlignmentPlan { seq_index: 0, .. })
        ));

        placed.alignment_padding.insert(0, 0);
        placed.alignment_padding.insert(7, 0);
        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::ExtraAlignmentPlan { seq_index: 7, .. })
        ));
    }
}
