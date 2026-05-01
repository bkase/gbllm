//! Canonical LR35902 instruction and section encoder.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::isa::Instr;
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
    pub sub_index: u16,
    pub kind: EncodedItemKind,
    pub offset: u16,
    pub len: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
            .map(|item| (item.order(), SectionEncodeItem::Label)),
    );
    items.extend(
        section
            .instrs
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.order(), SectionEncodeItem::Instr(idx))),
    );
    items.extend(
        section
            .data_blocks
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.order(), SectionEncodeItem::DataBlock(idx))),
    );
    items.extend(
        section
            .alignments
            .iter()
            .enumerate()
            .map(|(idx, item)| (item.order(), SectionEncodeItem::Align(idx))),
    );
    items.sort_by_key(|(order, _)| *order);

    let mut bytes = Vec::with_capacity(usize::from(placed.final_size));
    let mut item_spans = Vec::new();
    let mut seen_alignments = std::collections::BTreeSet::new();

    for (order, item) in items {
        match item {
            SectionEncodeItem::Label => {}
            SectionEncodeItem::Instr(idx) => {
                let instr = &section.instrs[idx].data;
                let encoded = encode_instr(instr)?;
                let offset = checked_offset(bytes.len(), section.id)?;
                bytes.extend_from_slice(&encoded);
                item_spans.push(EncodedItemSpan {
                    seq_index: order.seq_index,
                    sub_index: order.sub_index,
                    kind: EncodedItemKind::Instr,
                    offset,
                    len: checked_span_len(encoded.len(), section.id, order.seq_index)?,
                });
            }
            SectionEncodeItem::DataBlock(idx) => {
                let offset = checked_offset(bytes.len(), section.id)?;
                let before = bytes.len();
                encode_data_block(&section.data_blocks[idx].data, &mut bytes);
                item_spans.push(EncodedItemSpan {
                    seq_index: order.seq_index,
                    sub_index: order.sub_index,
                    kind: EncodedItemKind::DataBlock,
                    offset,
                    len: checked_span_len(bytes.len() - before, section.id, order.seq_index)?,
                });
            }
            SectionEncodeItem::Align(idx) => {
                let align = &section.alignments[idx];
                let align_order = align.order();
                seen_alignments.insert(align_order);
                let padding = *placed.alignment_padding.get(&align_order).ok_or(
                    EncodeError::MissingAlignmentPlan {
                        section_id: section.id,
                        seq_index: align.seq_index,
                    },
                )?;
                let offset = checked_offset(bytes.len(), section.id)?;
                bytes.resize(bytes.len() + usize::from(padding), PAD_BYTE);
                item_spans.push(EncodedItemSpan {
                    seq_index: order.seq_index,
                    sub_index: order.sub_index,
                    kind: EncodedItemKind::AlignmentPadding,
                    offset,
                    len: padding,
                });
            }
        }
    }

    for order in placed.alignment_padding.keys() {
        if !seen_alignments.contains(order) {
            return Err(EncodeError::ExtraAlignmentPlan {
                section_id: section.id,
                seq_index: order.seq_index,
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
///
/// Thin wrapper around [`Instr::describe`] — the canonical per-variant
/// byte/mnemonic dispatch lives in `isa`.
pub fn encode_instr(instr: &Instr) -> Result<Vec<u8>, EncodeError> {
    let bytes = instr.describe(0).bytes;
    if bytes.len() != usize::from(instr.byte_len()) {
        return Err(EncodeError::EncodedLengthMismatch {
            expected: instr.byte_len(),
            actual: bytes.len() as u8,
            instr: *instr,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU16;

    use super::*;
    use crate::isa::Instr;
    use crate::layout::{AddressSpace, BankIndex};
    use crate::provenance::{InstrProvenance, PlanningStage};
    use crate::section::{
        Align, Label, LegalizedSection, OrderedItem, SectionId, SectionRole, SymbolId,
    };
    use crate::symbols::SymbolName;
    use crate::test_support::gbdev_instr_cases;

    fn enc(instr: Instr) -> Vec<u8> {
        encode_instr(&instr).expect("instruction encodes")
    }

    #[test]
    fn unprefixed_opcodes_match_gbdev_json() {
        let cases = gbdev_instr_cases();
        let mut count = 0;
        for case in cases.iter().filter(|case| !case.is_prefixed()) {
            let instr = case.instr();
            assert_eq!(enc(instr), case.expected_bytes(), "{}", case.label());
            assert_eq!(
                instr.byte_len(),
                case.expected_byte_len(),
                "{}",
                case.label()
            );
            count += 1;
        }
        assert_eq!(count, 244);
    }

    #[test]
    fn cb_prefixed_opcodes_match_gbdev_json() {
        let cases = gbdev_instr_cases();
        let mut count = 0;
        for case in cases.iter().filter(|case| case.is_prefixed()) {
            let instr = case.instr();
            assert_eq!(enc(instr), case.expected_bytes(), "{}", case.label());
            assert_eq!(
                instr.byte_len(),
                case.expected_byte_len(),
                "{}",
                case.label()
            );
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn encode_section_merges_legalized_arrays_in_order() {
        let prov = InstrProvenance::new(PlanningStage::Backend);
        let section = LegalizedSection {
            id: SectionId::new(9),
            role: SectionRole::HeaderCartridge,
            name: SymbolName::runtime("test", "section").expect("symbol"),
            privilege: crate::section::SectionPrivilege::normal(),
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
        alignment_padding.insert(section.alignments[0].order(), 3);
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
                    sub_index: 0,
                    kind: EncodedItemKind::Instr,
                    offset: 0,
                    len: 1,
                },
                EncodedItemSpan {
                    seq_index: 2,
                    sub_index: 0,
                    kind: EncodedItemKind::AlignmentPadding,
                    offset: 1,
                    len: 3,
                },
                EncodedItemSpan {
                    seq_index: 3,
                    sub_index: 0,
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
            privilege: crate::section::SectionPrivilege::normal(),
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
            privilege: crate::section::SectionPrivilege::normal(),
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

        placed
            .alignment_padding
            .insert(section.alignments[0].order(), 0);
        placed
            .alignment_padding
            .insert(crate::section::ItemOrder::new(7, 0), 0);
        assert!(matches!(
            encode_section(&section, &placed),
            Err(EncodeError::ExtraAlignmentPlan { seq_index: 7, .. })
        ));
    }
}
