//! Once-per-frame JOYP reader with active-low decode into a WRAM cache.

use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, Instr, Reg8};
use gbf_asm::section::{Section, SectionRole};
use gbf_asm::symbols::SymbolName;

use crate::{SECTION_ID_JOYPAD, WramAddr};

pub const JOYPAD_CACHED_STATE_ADDR: WramAddr = WramAddr::new(0xC100);
pub const JOYPAD_PREV_STATE_ADDR: WramAddr = WramAddr::new(0xC101);

pub fn build_joypad_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_JOYPAD,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("joypad", "section").expect("static symbol"),
    );
    builder.label(SymbolName::runtime("joypad", "read").expect("static symbol"));
    emit_joypad_read(&mut builder);
    builder.emit(Instr::Ret { cond: None });
    builder.finish().with_size_hint_bytes(96)
}

pub fn emit_joypad_read(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(JOYPAD_CACHED_STATE_ADDR.get()),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(JOYPAD_PREV_STATE_ADDR.get()),
    });

    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: gbf_hw::joypad::JOYP_SELECT_DIRECTIONS,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: joyp_offset(),
    });
    b.emit(Instr::LdAFromHighDirect {
        offset: joyp_offset(),
    });
    b.emit(Instr::LdAFromHighDirect {
        offset: joyp_offset(),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::JOYP_INPUT_MASK),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::B,
        imm: 0,
    });
    emit_copy_direction_bit(b, BitIndex::B2, BitIndex::B4);
    emit_copy_direction_bit(b, BitIndex::B3, BitIndex::B5);
    emit_copy_direction_bit(b, BitIndex::B1, BitIndex::B6);
    emit_copy_direction_bit(b, BitIndex::B0, BitIndex::B7);

    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: gbf_hw::joypad::JOYP_SELECT_BUTTONS,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: joyp_offset(),
    });
    b.emit(Instr::LdAFromHighDirect {
        offset: joyp_offset(),
    });
    b.emit(Instr::LdAFromHighDirect {
        offset: joyp_offset(),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::JOYP_INPUT_MASK),
    });
    b.emit(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    b.emit(Instr::Cpl);
    b.emit(Instr::LdDirectFromA {
        addr: direct(JOYPAD_CACHED_STATE_ADDR.get()),
    });

    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: gbf_hw::joypad::JOYP_SELECT_BUTTONS | gbf_hw::joypad::JOYP_SELECT_DIRECTIONS,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: joyp_offset(),
    });
}

fn emit_copy_direction_bit(b: &mut Builder, source: BitIndex, dest: BitIndex) {
    b.emit(Instr::Bit {
        bit: source,
        target: CbTarget::Reg(Reg8::C),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::Z),
        off: 2,
    });
    b.emit(Instr::Set {
        bit: dest,
        target: CbTarget::Reg(Reg8::B),
    });
}

#[must_use]
pub const fn joypad_isr_is_no_op() -> bool {
    true
}

fn joyp_offset() -> HighDirectOffset {
    HighDirectOffset::new((gbf_hw::joypad::JOYP_REGISTER & 0x00FF) as u8)
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("joypad WRAM address is below high memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_asm::isa::Instr;

    #[test]
    fn read_emits_expected_sequence() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "joypad").unwrap(),
        );
        emit_joypad_read(&mut builder);
        let instrs: Vec<_> = builder.finish().instrs().iter().map(|i| i.data).collect();
        assert!(matches!(
            instrs[2],
            Instr::Ld8RegFromImm {
                dst: Reg8::A,
                imm: gbf_hw::joypad::JOYP_SELECT_DIRECTIONS
            }
        ));
        assert!(instrs.iter().any(|instr| matches!(
            instr,
            Instr::Bit {
                bit: BitIndex::B2,
                target: CbTarget::Reg(Reg8::C)
            }
        )));
        assert!(instrs.iter().any(|instr| matches!(
            instr,
            Instr::Set {
                bit: BitIndex::B7,
                target: CbTarget::Reg(Reg8::B)
            }
        )));
        assert!(matches!(
            instrs.last().copied(),
            Some(Instr::LdHighDirectFromA { .. })
        ));
    }

    #[test]
    fn cached_state_addr_in_wram() {
        assert!(gbf_hw::memory::is_wram(JOYPAD_CACHED_STATE_ADDR.get()));
        assert!(gbf_hw::memory::is_wram(JOYPAD_PREV_STATE_ADDR.get()));
    }

    #[test]
    fn cache_write_uses_absolute_load() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "joypad").unwrap(),
        );
        emit_joypad_read(&mut builder);
        let section = builder.finish();
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdDirectFromA { addr } if addr.get() == JOYPAD_CACHED_STATE_ADDR.get())
        }));
        assert!(!section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdHighDirectFromA { offset } if offset.absolute_addr() == JOYPAD_CACHED_STATE_ADDR.get())
        }));
    }

    #[test]
    fn isr_is_no_op() {
        assert!(joypad_isr_is_no_op());
    }
}
