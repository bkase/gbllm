//! Deterministic human-readable listings for encoded sections.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::cycle_model::{CycleCost, cycle_cost};
use crate::encoder::{EncodedItemKind, EncodedItemSpan, EncodedSection};
use crate::isa::{
    AluSrc8, CbTarget, Cond, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data, Reg16Stack,
};
use crate::layout::{LayoutPlan, PlacedSection};
use crate::provenance::InstrProvenance;
use crate::section::{DataBlock, ItemOrder, LegalizedSection, OrderedItem, SectionId};
use crate::symbols::{SymbolAddress, SymbolTable};

const DATA_CHUNK: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingOptions {
    pub show_provenance: bool,
    pub show_cycle_costs: bool,
    pub show_bytes: bool,
    pub include_section_header: bool,
    pub address_radix: AddressRadix,
}

impl Default for ListingOptions {
    fn default() -> Self {
        Self {
            show_provenance: true,
            show_cycle_costs: false,
            show_bytes: true,
            include_section_header: true,
            address_radix: AddressRadix::Hex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressRadix {
    Hex,
    Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingError {
    MissingSection { section_id: SectionId },
    MissingPlacement { section_id: SectionId },
}

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSection { section_id } => {
                write!(
                    f,
                    "encoded section {} has no legalized section",
                    section_id.get()
                )
            }
            Self::MissingPlacement { section_id } => {
                write!(f, "section {} has no placement", section_id.get())
            }
        }
    }
}

impl std::error::Error for ListingError {}

/// Emits a deterministic listing for one encoded section.
#[must_use]
pub fn emit_listing(
    section: &LegalizedSection,
    encoded: &EncodedSection,
    placed: &PlacedSection,
    symbols: &SymbolTable,
    opts: &ListingOptions,
) -> String {
    let mut out = String::new();
    if opts.include_section_header {
        out.push_str(&format!(
            "; section: {} ({:?})\n; bank={} origin={} size=0x{:04X}\n",
            section.name,
            section.role,
            placed.bank,
            format_addr(placed.cpu_start, opts.address_radix),
            placed.final_size
        ));
    }

    let spans = span_map(&encoded.item_spans);
    let mut emitted_symbol_offsets = BTreeSet::new();
    for item in ordered_items(section) {
        match item {
            ListingItem::Label(label) => {
                if let Some(offset) = symbols.resolve(&label.data.name).and_then(|addr| {
                    (addr.section == section.id).then_some(u16::try_from(addr.offset).ok()?)
                }) {
                    emit_symbol_lines(
                        &mut out,
                        section.id,
                        offset,
                        placed.cpu_start,
                        symbols,
                        opts.address_radix,
                        &mut emitted_symbol_offsets,
                    );
                }
            }
            ListingItem::Instr(instr) => {
                let Some(span) = spans.get(&(instr.order(), EncodedItemKind::Instr)) else {
                    continue;
                };
                emit_symbol_lines(
                    &mut out,
                    section.id,
                    span.offset,
                    placed.cpu_start,
                    symbols,
                    opts.address_radix,
                    &mut emitted_symbol_offsets,
                );
                let bytes = span_bytes(encoded, *span);
                let addr = placed.cpu_start.wrapping_add(span.offset);
                let mnemonic = format_instr(&instr.data, addr, symbols);
                out.push_str(&format_record(
                    addr,
                    &bytes,
                    &mnemonic,
                    &instr.provenance,
                    Some(cycle_cost(&instr.data)),
                    opts,
                ));
            }
            ListingItem::DataBlock(block) => {
                let Some(span) = spans.get(&(block.order(), EncodedItemKind::DataBlock)) else {
                    continue;
                };
                emit_symbol_lines(
                    &mut out,
                    section.id,
                    span.offset,
                    placed.cpu_start,
                    symbols,
                    opts.address_radix,
                    &mut emitted_symbol_offsets,
                );
                emit_chunked_record(
                    &mut out,
                    placed.cpu_start,
                    span.offset,
                    span_bytes(encoded, *span),
                    &format!("db {} bytes", span.len),
                    &block.provenance,
                    opts,
                );
            }
            ListingItem::Align(align) => {
                let Some(span) = spans.get(&(align.order(), EncodedItemKind::AlignmentPadding))
                else {
                    continue;
                };
                emit_symbol_lines(
                    &mut out,
                    section.id,
                    span.offset,
                    placed.cpu_start,
                    symbols,
                    opts.address_radix,
                    &mut emitted_symbol_offsets,
                );
                emit_chunked_record(
                    &mut out,
                    placed.cpu_start,
                    span.offset,
                    span_bytes(encoded, *span),
                    &format!("align {} padding", align.data.0),
                    &align.provenance,
                    opts,
                );
            }
        }
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn emit_program_listing(
    sections: &[LegalizedSection],
    encoded_sections: &[EncodedSection],
    layout: &LayoutPlan,
    symbols: &SymbolTable,
    opts: &ListingOptions,
) -> Result<String, ListingError> {
    let mut out = String::new();
    for encoded in encoded_sections {
        let section = sections
            .iter()
            .find(|candidate| candidate.id == encoded.id)
            .ok_or(ListingError::MissingSection {
                section_id: encoded.id,
            })?;
        let placed = layout
            .sections
            .iter()
            .find(|candidate| candidate.id == encoded.id)
            .ok_or(ListingError::MissingPlacement {
                section_id: encoded.id,
            })?;
        out.push_str(&emit_listing(section, encoded, placed, symbols, opts));
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum ListingItem<'a> {
    Label(&'a OrderedItem<crate::section::Label>),
    Instr(&'a OrderedItem<Instr>),
    DataBlock(&'a OrderedItem<DataBlock>),
    Align(&'a OrderedItem<crate::section::Align>),
}

impl ListingItem<'_> {
    const fn order(&self) -> ItemOrder {
        match self {
            Self::Label(item) => item.order(),
            Self::Instr(item) => item.order(),
            Self::DataBlock(item) => item.order(),
            Self::Align(item) => item.order(),
        }
    }
}

fn ordered_items(section: &LegalizedSection) -> Vec<ListingItem<'_>> {
    let mut items = Vec::with_capacity(
        section.labels.len()
            + section.instrs.len()
            + section.data_blocks.len()
            + section.alignments.len(),
    );
    items.extend(section.labels.iter().map(ListingItem::Label));
    items.extend(section.instrs.iter().map(ListingItem::Instr));
    items.extend(section.data_blocks.iter().map(ListingItem::DataBlock));
    items.extend(section.alignments.iter().map(ListingItem::Align));
    items.sort_by_key(ListingItem::order);
    items
}

fn span_map(spans: &[EncodedItemSpan]) -> BTreeMap<(ItemOrder, EncodedItemKind), EncodedItemSpan> {
    spans
        .iter()
        .copied()
        .map(|span| {
            (
                (ItemOrder::new(span.seq_index, span.sub_index), span.kind),
                span,
            )
        })
        .collect()
}

fn span_bytes(encoded: &EncodedSection, span: EncodedItemSpan) -> Vec<u8> {
    let start = usize::from(span.offset);
    let end = start + usize::from(span.len);
    encoded.bytes[start..end].to_vec()
}

fn emit_symbol_lines(
    out: &mut String,
    section_id: SectionId,
    offset: u16,
    cpu_start: u16,
    symbols: &SymbolTable,
    radix: AddressRadix,
    emitted: &mut BTreeSet<u16>,
) {
    if !emitted.insert(offset) {
        return;
    }
    let address = SymbolAddress::new(section_id, u32::from(offset));
    let names = symbols.names_for(address);
    if names.is_empty() {
        return;
    }
    let cpu = cpu_start.wrapping_add(offset);
    for name in names {
        out.push_str(&format!("{}  <{}>:\n", format_addr(cpu, radix), name));
    }
}

fn emit_chunked_record(
    out: &mut String,
    section_start: u16,
    offset: u16,
    bytes: Vec<u8>,
    label: &str,
    provenance: &InstrProvenance,
    opts: &ListingOptions,
) {
    if bytes.is_empty() {
        let addr = section_start.wrapping_add(offset);
        out.push_str(&format_record(addr, &[], label, provenance, None, opts));
        return;
    }
    for (idx, chunk) in bytes.chunks(DATA_CHUNK).enumerate() {
        let addr = section_start.wrapping_add(offset + (idx * DATA_CHUNK) as u16);
        let text = if idx == 0 {
            label.to_owned()
        } else {
            "continued".to_owned()
        };
        out.push_str(&format_record(addr, chunk, &text, provenance, None, opts));
    }
}

fn format_record(
    addr: u16,
    bytes: &[u8],
    text: &str,
    provenance: &InstrProvenance,
    cycles: Option<CycleCost>,
    opts: &ListingOptions,
) -> String {
    let byte_col = if opts.show_bytes {
        format!("{:<47}", format_bytes(bytes))
    } else {
        String::new()
    };
    let mut suffix = Vec::new();
    if opts.show_cycle_costs
        && let Some(cycles) = cycles
    {
        suffix.push(format!("cycles={}", format_cycles(cycles)));
    }
    if opts.show_provenance {
        suffix.push(format_provenance(provenance));
    }
    let suffix = if suffix.is_empty() {
        String::new()
    } else {
        format!("  ; {}", suffix.join(" "))
    };
    if opts.show_bytes {
        format!(
            "{}  {} ; {:<24}{}\n",
            format_addr(addr, opts.address_radix),
            byte_col,
            text,
            suffix
        )
    } else {
        format!(
            "{}  ; {:<24}{}\n",
            format_addr(addr, opts.address_radix),
            text,
            suffix
        )
    }
}

fn format_addr(addr: u16, radix: AddressRadix) -> String {
    match radix {
        AddressRadix::Hex => format!("${addr:04X}"),
        AddressRadix::Decimal => format!("{addr:05}"),
    }
}

fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_provenance(provenance: &InstrProvenance) -> String {
    let mut out = format!("stage={}", provenance.stage.canonical_name());
    if let Some(op) = &provenance.source_op {
        out.push_str(&format!(" op={op}"));
    }
    if let Some(node) = provenance.source_node {
        out.push_str(&format!(" node={node}"));
    }
    if let Some(note) = &provenance.note {
        out.push_str(&format!(" note={note}"));
    }
    out
}

fn format_cycles(cost: CycleCost) -> String {
    match cost {
        CycleCost::Fixed(cycles) => cycles.get().to_string(),
        CycleCost::Branch { taken, not_taken } => {
            format!("{}/{}", taken.get(), not_taken.get())
        }
    }
}

/// Formats one canonical LR35902 instruction mnemonic.
#[must_use]
pub fn format_instr(instr: &Instr, here: u16, _symbols: &SymbolTable) -> String {
    match *instr {
        Instr::Nop => "nop".to_owned(),
        Instr::Stop => "stop".to_owned(),
        Instr::Halt => "halt".to_owned(),
        Instr::Di => "di".to_owned(),
        Instr::Ei => "ei".to_owned(),
        Instr::Ccf => "ccf".to_owned(),
        Instr::Scf => "scf".to_owned(),
        Instr::Cpl => "cpl".to_owned(),
        Instr::Daa => "daa".to_owned(),
        Instr::Ld8Reg { dst, src } => format!("ld   {}, {}", reg8(dst), reg8(src)),
        Instr::Ld8RegFromImm { dst, imm } => format!("ld   {}, {}", reg8(dst), hex8(imm)),
        Instr::Ld8RegFromHl { dst } => format!("ld   {}, (hl)", reg8(dst)),
        Instr::Ld8HlFromReg { src } => format!("ld   (hl), {}", reg8(src)),
        Instr::Ld8HlFromImm { imm } => format!("ld   (hl), {}", hex8(imm)),
        Instr::LdAFromReg16Addr { src } => format!("ld   a, {}", reg16_addr_mem(src)),
        Instr::LdReg16AddrFromA { dst } => format!("ld   {}, a", reg16_addr_mem(dst)),
        Instr::LdAFromDirect { addr } => format!("ld   a, ({})", hex16(addr.get())),
        Instr::LdDirectFromA { addr } => format!("ld   ({}), a", hex16(addr.get())),
        Instr::LdAFromHighDirect { offset } => {
            format!("ldh  a, ({})", hex8(offset.get()))
        }
        Instr::LdHighDirectFromA { offset } => {
            format!("ldh  ({}), a", hex8(offset.get()))
        }
        Instr::LdAFromHighC => "ldh  a, (c)".to_owned(),
        Instr::LdHighCFromA => "ldh  (c), a".to_owned(),
        Instr::Ld16Imm { dst, imm } => format!("ld   {}, {}", reg16_data(dst), hex16(imm)),
        Instr::LdSpFromHl => "ld   sp, hl".to_owned(),
        Instr::LdDirectFromSp { addr } => format!("ld   ({}), sp", hex16(addr)),
        Instr::LdHlFromSpPlus { off } => format!("ld   hl, sp{:+}", off),
        Instr::AddA { src } => format!("add  a, {}", alu_src(src)),
        Instr::AdcA { src } => format!("adc  a, {}", alu_src(src)),
        Instr::SubA { src } => format!("sub  a, {}", alu_src(src)),
        Instr::SbcA { src } => format!("sbc  a, {}", alu_src(src)),
        Instr::AndA { src } => format!("and  a, {}", alu_src(src)),
        Instr::OrA { src } => format!("or   a, {}", alu_src(src)),
        Instr::XorA { src } => format!("xor  a, {}", alu_src(src)),
        Instr::CpA { src } => format!("cp   a, {}", alu_src(src)),
        Instr::Inc8 { dst } => format!("inc  {}", inc_dec8(dst)),
        Instr::Dec8 { dst } => format!("dec  {}", inc_dec8(dst)),
        Instr::Inc16 { dst } => format!("inc  {}", reg16_data(dst)),
        Instr::Dec16 { dst } => format!("dec  {}", reg16_data(dst)),
        Instr::AddHl { src } => format!("add  hl, {}", reg16_data(src)),
        Instr::AddSp { off } => format!("add  sp, {:+}", off),
        Instr::Rlca => "rlca".to_owned(),
        Instr::Rrca => "rrca".to_owned(),
        Instr::Rla => "rla".to_owned(),
        Instr::Rra => "rra".to_owned(),
        Instr::Rlc { target } => format!("rlc  {}", cb_target(target)),
        Instr::Rrc { target } => format!("rrc  {}", cb_target(target)),
        Instr::Rl { target } => format!("rl   {}", cb_target(target)),
        Instr::Rr { target } => format!("rr   {}", cb_target(target)),
        Instr::Sla { target } => format!("sla  {}", cb_target(target)),
        Instr::Sra { target } => format!("sra  {}", cb_target(target)),
        Instr::Srl { target } => format!("srl  {}", cb_target(target)),
        Instr::Swap { target } => format!("swap {}", cb_target(target)),
        Instr::Bit { bit, target } => format!("bit  {}, {}", bit.get(), cb_target(target)),
        Instr::Res { bit, target } => format!("res  {}, {}", bit.get(), cb_target(target)),
        Instr::Set { bit, target } => format!("set  {}, {}", bit.get(), cb_target(target)),
        Instr::JpAbs { cond, addr } => branch_abs("jp", cond, addr),
        Instr::JpHl => "jp   hl".to_owned(),
        Instr::JrRel { cond, off } => {
            let target = here.wrapping_add(2).wrapping_add_signed(i16::from(off));
            branch_rel("jr", cond, off, target)
        }
        Instr::Call { cond, addr } => branch_abs("call", cond, addr),
        Instr::Ret { cond } => match cond {
            None => "ret".to_owned(),
            Some(cond) => format!("ret  {}", cond_name(cond)),
        },
        Instr::Reti => "reti".to_owned(),
        Instr::Rst { vector } => format!("rst  {}", hex8(vector.addr())),
        Instr::Push { src } => format!("push {}", reg16_stack(src)),
        Instr::Pop { dst } => format!("pop  {}", reg16_stack(dst)),
    }
}

fn branch_abs(op: &str, cond: Option<Cond>, addr: u16) -> String {
    match cond {
        None => format!("{op:<4} {}", hex16(addr)),
        Some(cond) => format!("{op:<4} {}, {}", cond_name(cond), hex16(addr)),
    }
}

fn branch_rel(op: &str, cond: Option<Cond>, off: i8, target: u16) -> String {
    match cond {
        None => format!("{op:<4} {:+} ({})", off, hex16(target)),
        Some(cond) => format!("{op:<4} {}, {:+} ({})", cond_name(cond), off, hex16(target)),
    }
}

fn reg8(reg: Reg8) -> &'static str {
    match reg {
        Reg8::A => "a",
        Reg8::B => "b",
        Reg8::C => "c",
        Reg8::D => "d",
        Reg8::E => "e",
        Reg8::H => "h",
        Reg8::L => "l",
    }
}

fn reg16_data(reg: Reg16Data) -> &'static str {
    match reg {
        Reg16Data::BC => "bc",
        Reg16Data::DE => "de",
        Reg16Data::HL => "hl",
        Reg16Data::SP => "sp",
    }
}

fn reg16_stack(reg: Reg16Stack) -> &'static str {
    match reg {
        Reg16Stack::BC => "bc",
        Reg16Stack::DE => "de",
        Reg16Stack::HL => "hl",
        Reg16Stack::AF => "af",
    }
}

fn reg16_addr_mem(reg: Reg16Addr) -> &'static str {
    match reg {
        Reg16Addr::BC => "(bc)",
        Reg16Addr::DE => "(de)",
        Reg16Addr::Hli => "(hl+)",
        Reg16Addr::Hld => "(hl-)",
    }
}

fn alu_src(src: AluSrc8) -> String {
    match src {
        AluSrc8::Reg(reg) => reg8(reg).to_owned(),
        AluSrc8::HlIndirect => "(hl)".to_owned(),
        AluSrc8::Imm(imm) => hex8(imm),
    }
}

fn inc_dec8(target: IncDec8Target) -> &'static str {
    match target {
        IncDec8Target::Reg(reg) => reg8(reg),
        IncDec8Target::HlIndirect => "(hl)",
    }
}

fn cb_target(target: CbTarget) -> &'static str {
    match target {
        CbTarget::Reg(reg) => reg8(reg),
        CbTarget::HlIndirect => "(hl)",
    }
}

fn cond_name(cond: Cond) -> &'static str {
    match cond {
        Cond::NZ => "nz",
        Cond::Z => "z",
        Cond::NC => "nc",
        Cond::C => "c",
    }
}

fn hex8(value: u8) -> String {
    format!("${value:02X}")
}

fn hex16(value: u16) -> String {
    format!("${value:04X}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU16;

    use super::*;
    use crate::encoder::{EncodedItemKind, PAD_BYTE, encode_section};
    use crate::isa::{BitIndex, Cond, HighDirectOffset, Reg8, Reg16Data};
    use crate::layout::{AddressSpace, BankIndex};
    use crate::provenance::{InstrProvenance, PlanningStage};
    use crate::section::{Align, Label, LegalizedSection, OrderedItem, SectionRole, SymbolId};
    use crate::symbols::SymbolName;

    fn prov(op: &'static str) -> InstrProvenance {
        InstrProvenance::new(PlanningStage::Backend).with_source_op(op)
    }

    fn fixture_section(data: Vec<u8>, padding: u16) -> (LegalizedSection, PlacedSection) {
        let label = SymbolName::runtime("listing", "entry").expect("symbol");
        let section = LegalizedSection {
            id: SectionId::new(1),
            role: SectionRole::Bank0Data,
            name: SymbolName::runtime("listing", "section").expect("symbol"),
            privilege: crate::section::SectionPrivilege::normal(),
            align: NonZeroU16::new(1).expect("nonzero"),
            size_hint_bytes: None,
            next_seq_index: 5,
            labels: vec![OrderedItem::new(
                Label {
                    id: SymbolId::new(0),
                    name: label,
                },
                0,
                prov("label"),
            )],
            instrs: vec![OrderedItem::new(
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm: 0x42,
                },
                1,
                prov("load"),
            )],
            data_blocks: vec![OrderedItem::new(DataBlock::Bytes(data), 2, prov("data"))],
            alignments: vec![OrderedItem::new(
                Align(NonZeroU16::new(16).expect("nonzero")),
                3,
                prov("align"),
            )],
        };
        let final_size = 2 + section.data_blocks[0].data.byte_len() as u16 + padding;
        let mut alignment_padding = BTreeMap::new();
        alignment_padding.insert(section.alignments[0].order(), padding);
        let placed = PlacedSection {
            id: section.id,
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start: 0x0150,
            final_size,
            estimated_size: final_size,
            alignment_padding,
        };
        (section, placed)
    }

    fn symbols_for(section: &LegalizedSection) -> SymbolTable {
        let mut symbols = SymbolTable::new();
        symbols
            .insert(
                section.labels[0].data.name.clone(),
                SymbolAddress::new(section.id, 0),
            )
            .expect("insert symbol");
        symbols
    }

    #[test]
    fn byte_stable() {
        let (section, placed) = fixture_section(vec![1, 2, 3], 4);
        let encoded = encode_section(&section, &placed).expect("encode");
        let symbols = symbols_for(&section);
        let opts = ListingOptions::default();
        assert_eq!(
            emit_listing(&section, &encoded, &placed, &symbols, &opts),
            emit_listing(&section, &encoded, &placed, &symbols, &opts)
        );
    }

    #[test]
    fn all_options_render() {
        let (section, placed) = fixture_section(vec![1, 2, 3], 4);
        let encoded = encode_section(&section, &placed).expect("encode");
        let symbols = symbols_for(&section);
        let base = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols,
            &ListingOptions::default(),
        );
        let no_prov = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols,
            &ListingOptions {
                show_provenance: false,
                ..ListingOptions::default()
            },
        );
        let cycles = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols,
            &ListingOptions {
                show_cycle_costs: true,
                ..ListingOptions::default()
            },
        );
        let no_bytes = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols,
            &ListingOptions {
                show_bytes: false,
                ..ListingOptions::default()
            },
        );
        let decimal = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols,
            &ListingOptions {
                address_radix: AddressRadix::Decimal,
                ..ListingOptions::default()
            },
        );
        assert_ne!(base, no_prov);
        assert_ne!(base, cycles);
        assert_ne!(base, no_bytes);
        assert_ne!(base, decimal);
    }

    #[test]
    fn provenance_visible() {
        let (section, placed) = fixture_section(vec![1], 0);
        let encoded = encode_section(&section, &placed).expect("encode");
        let listing = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols_for(&section),
            &ListingOptions::default(),
        );
        assert!(listing.contains("stage=backend"));
        assert!(listing.contains("op=load"));
    }

    #[test]
    fn format_instr_canonical() {
        let symbols = SymbolTable::new();
        let cases = [
            (Instr::Nop, "nop"),
            (
                Instr::Ld8Reg {
                    dst: Reg8::A,
                    src: Reg8::B,
                },
                "ld   a, b",
            ),
            (
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm: 0x42,
                },
                "ld   a, $42",
            ),
            (
                Instr::Ld16Imm {
                    dst: Reg16Data::HL,
                    imm: 0xC000,
                },
                "ld   hl, $C000",
            ),
            (
                Instr::LdAFromHighDirect {
                    offset: HighDirectOffset::new(0x44),
                },
                "ldh  a, ($44)",
            ),
            (
                Instr::JpAbs {
                    cond: None,
                    addr: 0x0150,
                },
                "jp   $0150",
            ),
            (
                Instr::JpAbs {
                    cond: Some(Cond::NZ),
                    addr: 0x4000,
                },
                "jp   nz, $4000",
            ),
            (
                Instr::JrRel {
                    cond: None,
                    off: -2,
                },
                "jr   -2 ($4000)",
            ),
            (
                Instr::Call {
                    cond: None,
                    addr: 0x4000,
                },
                "call $4000",
            ),
            (
                Instr::Bit {
                    bit: BitIndex::B7,
                    target: CbTarget::Reg(Reg8::H),
                },
                "bit  7, h",
            ),
            (
                Instr::Swap {
                    target: CbTarget::HlIndirect,
                },
                "swap (hl)",
            ),
        ];
        for (instr, expected) in cases {
            assert_eq!(format_instr(&instr, 0x4000, &symbols), expected);
        }
    }

    #[test]
    fn large_data_block_is_chunked_deterministically() {
        let (section, placed) = fixture_section((0_u8..40).collect(), 0);
        let encoded = encode_section(&section, &placed).expect("encode");
        let listing = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols_for(&section),
            &ListingOptions::default(),
        );
        assert_eq!(listing.matches("db 40 bytes").count(), 1);
        assert_eq!(listing.matches("continued").count(), 2);
    }

    #[test]
    fn large_alignment_padding_is_chunked_deterministically() {
        let (section, placed) = fixture_section(vec![], 40);
        let encoded = encode_section(&section, &placed).expect("encode");
        let listing = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols_for(&section),
            &ListingOptions::default(),
        );
        assert_eq!(listing.matches("align 16 padding").count(), 1);
        assert_eq!(listing.matches("continued").count(), 2);
        assert!(encoded.bytes.ends_with(&[PAD_BYTE; 4]));
        assert!(
            encoded
                .item_spans
                .iter()
                .any(|span| { span.kind == EncodedItemKind::AlignmentPadding && span.len == 40 })
        );
    }

    #[test]
    fn cycle_cost_shown() {
        let (section, placed) = fixture_section(vec![1], 0);
        let encoded = encode_section(&section, &placed).expect("encode");
        let listing = emit_listing(
            &section,
            &encoded,
            &placed,
            &symbols_for(&section),
            &ListingOptions {
                show_cycle_costs: true,
                ..ListingOptions::default()
            },
        );
        assert!(listing.contains("cycles=2"));
    }
}
