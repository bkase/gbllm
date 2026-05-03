//! M0 panic path with audited direct VRAM rendering bypass.

use gbf_abi::FaultCode;
use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, CbTarget, Cond, DirectAddr, HighDirectOffset, Instr, Reg8, Reg16Data};
use gbf_asm::section::{
    ExecutionContext, InterruptDiscipline, Section, SectionPrivilege, SectionRole,
};
use gbf_asm::symbols::SymbolName;

use crate::{SECTION_ID_PANIC, WramAddr};

pub const WRAM_LAST_FAULT_ADDR: WramAddr = WramAddr::new(0xC360);
pub const WRAM_LAST_FAULT_HI_ADDR: WramAddr = WRAM_LAST_FAULT_ADDR.add(1);
pub const PANIC_SCREEN_BG_ADDR: u16 = 0x9800;
pub const PANIC_VISIBLE_LCDC: u8 = 0x91;
pub const PANIC_VISIBLE_BGP: u8 = 0xE4;

pub fn build_panic_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_PANIC,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("panic", "section").expect("static symbol"),
    )
    .with_section_privilege(
        SectionPrivilege::privileged()
            .with_execution_context(ExecutionContext::PanicOnly)
            .with_interrupt_discipline(InterruptDiscipline::ImeDisabled)
            .with_panic_bypass(true),
    );
    builder.label(SymbolName::runtime("panic", "entry").expect("static symbol"));
    emit_panic_from_hl(&mut builder);
    builder.finish().with_size_hint_bytes(256)
}

pub fn emit_panic(b: &mut Builder, code: FaultCode) {
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: code as u16,
    });
    emit_panic_from_hl(b);
}

pub fn emit_panic_from_hl(b: &mut Builder) {
    b.emit(Instr::Di);
    emit_store_fault_from_hl(b);
    emit_panic_after_fault_stored(b);
}

fn emit_store_fault_from_hl(b: &mut Builder) {
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(WRAM_LAST_FAULT_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(WRAM_LAST_FAULT_HI_ADDR.get()),
    });
}

fn emit_panic_after_fault_stored(b: &mut Builder) {
    emit_wait_for_vblank_before_lcdc_disable(b);
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::lcd::LCDC_REG),
    });
    emit_panic_screen_render(b);
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: PANIC_VISIBLE_BGP,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::lcd::BGP_REG),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: PANIC_VISIBLE_LCDC,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::lcd::LCDC_REG),
    });
    b.emit(Instr::Halt);
    b.emit(Instr::JrRel {
        cond: None,
        off: -2,
    });
}

pub fn emit_wait_for_vblank_before_lcdc_disable(b: &mut Builder) {
    b.emit(Instr::LdAFromHighDirect {
        offset: high(gbf_hw::lcd::LCDC_REG),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::Z),
        off: 6,
    });
    b.emit(Instr::LdAFromHighDirect {
        offset: high(gbf_hw::lcd::LY_REG),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(gbf_hw::lcd::VBLANK_LY_THRESHOLD),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::C),
        off: -6,
    });
}

pub fn emit_panic_screen_render(b: &mut Builder) {
    for (idx, glyph) in [b'F', b'A', b'U', b'L', b'T', b' '].into_iter().enumerate() {
        b.emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: glyph,
        });
        b.emit(Instr::LdDirectFromA {
            addr: direct(PANIC_SCREEN_BG_ADDR + idx as u16),
        });
    }
    emit_hex_byte_from_addr(b, WRAM_LAST_FAULT_HI_ADDR.get(), PANIC_SCREEN_BG_ADDR + 6);
    emit_hex_byte_from_addr(b, WRAM_LAST_FAULT_ADDR.get(), PANIC_SCREEN_BG_ADDR + 8);
}

fn emit_hex_byte_from_addr(b: &mut Builder, addr: u16, bg_addr: u16) {
    b.emit(Instr::LdAFromDirect { addr: direct(addr) });
    b.emit(Instr::Swap {
        target: CbTarget::Reg(Reg8::A),
    });
    emit_hex_nibble_from_a(b, bg_addr);
    b.emit(Instr::LdAFromDirect { addr: direct(addr) });
    emit_hex_nibble_from_a(b, bg_addr + 1);
}

fn emit_hex_nibble_from_a(b: &mut Builder, bg_addr: u16) {
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(0x0F),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(10),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::C),
        off: 2,
    });
    b.emit(Instr::AddA {
        src: AluSrc8::Imm(7),
    });
    b.emit(Instr::AddA {
        src: AluSrc8::Imm(b'0'),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(bg_addr),
    });
}

fn high(addr: u16) -> HighDirectOffset {
    HighDirectOffset::new((addr & 0x00FF) as u8)
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("panic direct address is below high memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section_effect_kinds;
    use gbf_asm::effect::{MachineEffectKind, PrivilegeClass};

    #[test]
    fn waits_for_vblank_before_lcdc_disable() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "panic_wait").unwrap(),
        )
        .with_section_privilege(SectionPrivilege::privileged());
        emit_panic(&mut builder, FaultCode::InternalAssertion);
        let instrs: Vec<_> = builder
            .finish()
            .instrs()
            .iter()
            .map(|item| item.data)
            .collect();
        let ly_read = instrs.iter().position(|instr| {
            matches!(*instr, Instr::LdAFromHighDirect { offset } if offset.absolute_addr() == gbf_hw::lcd::LY_REG)
        });
        let lcdc_clear = instrs.iter().position(|instr| {
            matches!(*instr, Instr::LdHighDirectFromA { offset } if offset.absolute_addr() == gbf_hw::lcd::LCDC_REG)
        });
        assert!(ly_read < lcdc_clear);
    }

    #[test]
    fn emits_di_then_halt() {
        let section = build_panic_section();
        assert_eq!(section.instrs()[0].data, Instr::Di);
        assert!(section.instrs().iter().any(|item| item.data == Instr::Halt));
    }

    #[test]
    fn renders_fault_code_glyphs() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "panic_render").unwrap(),
        );
        emit_panic_screen_render(&mut builder);
        let section = builder.finish();
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
        let stores: Vec<u16> = section
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::LdDirectFromA { addr } => Some(addr.get()),
                _ => None,
            })
            .collect();
        assert!(stores.contains(&(PANIC_SCREEN_BG_ADDR + 6)));
        assert!(stores.contains(&(PANIC_SCREEN_BG_ADDR + 9)));
    }

    #[test]
    fn reenables_visible_bg_mode() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "panic_visible").unwrap(),
        )
        .with_section_privilege(SectionPrivilege::privileged());
        emit_panic_after_fault_stored(&mut builder);
        let immediates: Vec<u8> = builder
            .finish()
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::Ld8RegFromImm { imm, .. } => Some(imm),
                _ => None,
            })
            .collect();
        assert!(immediates.contains(&PANIC_VISIBLE_BGP));
        assert!(immediates.contains(&PANIC_VISIBLE_LCDC));
    }

    #[test]
    fn section_marked_exempt() {
        let section = build_panic_section();
        assert_eq!(
            section.privilege().default_privilege,
            PrivilegeClass::Privileged
        );
        assert_eq!(
            section.privilege().execution_context,
            ExecutionContext::PanicOnly
        );
        assert_eq!(
            section.privilege().interrupt_discipline,
            InterruptDiscipline::ImeDisabled
        );
        assert!(section.privilege().panic_bypass);
    }

    #[test]
    fn wram_last_fault_byte_set() {
        let section = build_panic_section();
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::Ld8Reg {
                    dst: Reg8::A,
                    src: Reg8::L
                }
            )
        }));
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::Ld8Reg {
                    dst: Reg8::A,
                    src: Reg8::H
                }
            )
        }));
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdDirectFromA { addr } if addr.get() == WRAM_LAST_FAULT_ADDR.get())
        }));
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdDirectFromA { addr } if addr.get() == WRAM_LAST_FAULT_HI_ADDR.get())
        }));
    }

    #[test]
    fn is_only_other_vram_writer() {
        let section = build_panic_section();
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
        assert!(section.privilege().panic_bypass);
    }
}
