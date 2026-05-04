//! Bank0 boot entry, IRQ vector table, and ISR entry stubs.

use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, Cond, HighDirectOffset, Instr, Reg8, Reg16Stack};
use gbf_asm::section::{
    ExecutionContext, InterruptDiscipline, Section, SectionPrivilege, SectionRole, SymbolicBranch,
};
use gbf_asm::symbols::SymbolName;
use gbf_hw::interrupts::InterruptSource;
use serde::{Deserialize, Serialize};

use crate::banking;
use crate::scheduler::{
    HRAM_LDH_FRAME_COUNT, HRAM_LDH_PREV_CHECKPOINT_LO, HRAM_LDH_YIELD_REQUESTED,
};
use crate::{SECTION_ID_BOOT, SECTION_ID_IRQ_VECTORS, SECTION_ID_ISR_STUBS};

pub const RUNTIME_BOOT_ENTRY_ADDR: u16 = gbf_asm::rom::ENTRY_POINT;
pub const ISR_STUBS_BASE_ADDR: u16 = 0x0260;
pub const ISR_STUB_BYTES: u16 = 14;
pub const SCHEDULER_MAIN_LOOP_ADDR: u16 = 0x0300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BootInitPolicy {
    pub zero_hram_shadow: bool,
    pub power_up_lcd: bool,
    pub default_ie_mask: u8,
    pub default_lcdc: u8,
    pub default_stat: u8,
}

impl BootInitPolicy {
    #[must_use]
    pub const fn bring_up() -> Self {
        Self {
            zero_hram_shadow: true,
            power_up_lcd: true,
            default_ie_mask: gbf_hw::interrupts::ie_bit(InterruptSource::VBlank)
                | gbf_hw::interrupts::ie_bit(InterruptSource::LcdStat)
                | gbf_hw::interrupts::ie_bit(InterruptSource::Timer)
                | gbf_hw::interrupts::ie_bit(InterruptSource::Joypad),
            default_lcdc: 0x91,
            default_stat: gbf_hw::lcd::STAT_INTERRUPT_HBLANK_ENABLE,
        }
    }
}

impl Default for BootInitPolicy {
    fn default() -> Self {
        Self::bring_up()
    }
}

pub fn build_boot_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_BOOT,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("boot", "section").expect("static symbol"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(SymbolName::runtime("boot", "runtime_boot_entry").expect("static symbol"));
    emit_boot_init(&mut builder, BootInitPolicy::default());
    builder.branch(SymbolicBranch::jump(
        SymbolName::runtime("scheduler", "main_loop").expect("static symbol"),
        None,
    ));
    builder.finish().with_size_hint_bytes(160)
}

pub fn build_irq_vectors_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_IRQ_VECTORS,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("boot", "irq_vectors").expect("static symbol"),
    );
    for source in InterruptSource::ALL {
        builder.label(vector_label(source));
        builder.emit(Instr::JpAbs {
            cond: None,
            addr: isr_stub_addr(source),
        });
        for _ in 0..5 {
            builder.emit(Instr::Nop);
        }
    }
    builder.finish().with_size_hint_bytes(40)
}

pub fn build_isr_stubs_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_ISR_STUBS,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("interrupts", "isr_stubs").expect("static symbol"),
    )
    .with_section_privilege(
        SectionPrivilege::interrupt_handler()
            .with_execution_context(ExecutionContext::InterruptHandler)
            .with_interrupt_discipline(InterruptDiscipline::ImeDisabled),
    );

    for source in InterruptSource::ALL {
        builder.label(stub_label(source));
        emit_isr_stub_calling(&mut builder, source);
    }
    builder
        .finish()
        .with_size_hint_bytes(u32::from(ISR_STUB_BYTES) * 5)
}

pub fn emit_boot_init(b: &mut Builder, policy: BootInitPolicy) {
    b.emit(Instr::Di);
    if policy.zero_hram_shadow {
        banking::lower_banking_shadow_zero_init(b).expect("banking shadow zero-init emits");
        emit_fa5_hram_zero_init(b);
    }
    emit_lcdc_off_for_vram_bootstrap(b);
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("video_commit", "bootstrap_vram_init").expect("static symbol"),
        None,
    ));
    if policy.power_up_lcd {
        b.emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: policy.default_lcdc,
        });
        b.emit(Instr::LdHighDirectFromA {
            offset: high(gbf_hw::lcd::LCDC_REG),
        });
    }
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: policy.default_stat,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::lcd::STAT_REG),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: gbf_hw::interrupts::IE_IF_UNUSED_MASK,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::interrupts::IF_REGISTER),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: policy.default_ie_mask,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::interrupts::IE_REGISTER),
    });
    b.emit(Instr::Ei);
}

pub fn emit_lcdc_off_for_vram_bootstrap(b: &mut Builder) {
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
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::lcd::LCDC_REG),
    });
}

pub fn emit_fa5_hram_zero_init(b: &mut Builder) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    for offset in [
        HRAM_LDH_YIELD_REQUESTED,
        HRAM_LDH_FRAME_COUNT,
        HRAM_LDH_PREV_CHECKPOINT_LO,
        HRAM_LDH_PREV_CHECKPOINT_LO + 1,
    ] {
        b.emit(Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(offset),
        });
    }
}

pub fn emit_isr_stub_calling(b: &mut Builder, source: InterruptSource) {
    for reg in [
        Reg16Stack::AF,
        Reg16Stack::BC,
        Reg16Stack::DE,
        Reg16Stack::HL,
    ] {
        b.emit(Instr::Push { src: reg });
    }
    b.branch(SymbolicBranch::call(handler_label(source), None));
    for reg in [
        Reg16Stack::HL,
        Reg16Stack::DE,
        Reg16Stack::BC,
        Reg16Stack::AF,
    ] {
        b.emit(Instr::Pop { dst: reg });
    }
    b.emit(Instr::Reti);
}

#[must_use]
pub const fn isr_stub_addr(source: InterruptSource) -> u16 {
    ISR_STUBS_BASE_ADDR + (source as u16) * ISR_STUB_BYTES
}

fn vector_label(source: InterruptSource) -> SymbolName {
    SymbolName::runtime("boot", vector_name(source)).expect("static symbol")
}

fn stub_label(source: InterruptSource) -> SymbolName {
    SymbolName::runtime("interrupts", stub_name(source)).expect("static symbol")
}

fn handler_label(source: InterruptSource) -> SymbolName {
    SymbolName::runtime("interrupts", handler_name(source)).expect("static symbol")
}

const fn vector_name(source: InterruptSource) -> &'static str {
    match source {
        InterruptSource::VBlank => "vector_vblank",
        InterruptSource::LcdStat => "vector_lcd_stat",
        InterruptSource::Timer => "vector_timer",
        InterruptSource::Serial => "vector_serial",
        InterruptSource::Joypad => "vector_joypad",
    }
}

const fn stub_name(source: InterruptSource) -> &'static str {
    match source {
        InterruptSource::VBlank => "isr_vblank_stub",
        InterruptSource::LcdStat => "isr_lcd_stat_stub",
        InterruptSource::Timer => "isr_timer_stub",
        InterruptSource::Serial => "isr_serial_stub",
        InterruptSource::Joypad => "isr_joypad_stub",
    }
}

const fn handler_name(source: InterruptSource) -> &'static str {
    match source {
        InterruptSource::VBlank => "vblank_handler",
        InterruptSource::LcdStat => "lcd_stat_handler",
        InterruptSource::Timer => "timer_handler",
        InterruptSource::Serial => "serial_handler",
        InterruptSource::Joypad => "joypad_handler",
    }
}

fn high(addr: u16) -> HighDirectOffset {
    HighDirectOffset::new((addr & 0x00FF) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::HRAM_FAST_FLAGS_END_EXCLUSIVE;
    use gbf_asm::rom::{CartridgeHeader, assemble_rom};
    use gbf_asm::{
        encoder::EncodedSection,
        layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection},
    };
    use std::collections::BTreeMap;

    #[test]
    fn cartridge_header_layout() {
        let encoded = EncodedSection {
            id: SECTION_ID_BOOT,
            bytes: vec![0x76],
            item_spans: vec![],
        };
        let placed = PlacedSection {
            id: SECTION_ID_BOOT,
            space: AddressSpace::Rom0,
            bank: BankIndex::Rom(0),
            cpu_start: RUNTIME_BOOT_ENTRY_ADDR,
            final_size: 1,
            estimated_size: 1,
            alignment_padding: BTreeMap::new(),
        };
        let layout = LayoutPlan {
            sections: vec![placed.clone()],
            bank_count: 2,
            free_bytes_per_bank: BTreeMap::new(),
            reserved_ranges: Vec::new(),
        };
        let rom = assemble_rom(
            &[(encoded, placed)],
            &layout,
            &CartridgeHeader::new("GBFA5").unwrap(),
        )
        .expect("ROM assembles");
        assert_eq!(&rom[0x0100..=0x0103], &[0x00, 0xC3, 0x50, 0x01]);
        assert_eq!(
            &rom[0x0104..=0x0133],
            &gbf_hw::cartridge_header::NINTENDO_LOGO
        );
    }

    #[test]
    fn irq_vector_jumps() {
        let section = build_irq_vectors_section();
        assert_eq!(section.fixed_item_bytes(), Some(40));
        let jumps: Vec<u16> = section
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::JpAbs { addr, .. } => Some(addr),
                _ => None,
            })
            .collect();
        assert_eq!(
            jumps,
            InterruptSource::ALL
                .into_iter()
                .map(isr_stub_addr)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shadow_registers_zeroed_at_init() {
        let section = build_boot_section();
        let stores: Vec<u16> = section
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::LdHighDirectFromA { offset } => Some(offset.absolute_addr()),
                _ => None,
            })
            .collect();
        for addr in banking::HRAM_SHADOW_BASE..HRAM_FAST_FLAGS_END_EXCLUSIVE {
            assert!(
                stores.contains(&addr),
                "missing zero-init store for {addr:#06x}"
            );
        }
    }

    #[test]
    fn isr_stub_shape() {
        let section = build_isr_stubs_section();
        assert_eq!(section.instrs().len(), 9 * InterruptSource::ALL.len());
        assert_eq!(section.branches().len(), InterruptSource::ALL.len());
        assert!(
            section
                .instrs()
                .iter()
                .any(|item| matches!(item.data, Instr::Reti))
        );
    }

    #[test]
    fn boot_calls_bootstrap_before_lcdc_enable() {
        let section = build_boot_section();
        let mut call_order = None;
        let mut lcdc_orders = Vec::new();
        let mut ly_read_order = None;
        let bootstrap = SymbolName::runtime("video_commit", "bootstrap_vram_init").unwrap();
        for item in section.instrs() {
            match item.data {
                Instr::LdHighDirectFromA { offset }
                    if offset.absolute_addr() == gbf_hw::lcd::LCDC_REG =>
                {
                    lcdc_orders.push(item.order());
                }
                Instr::LdAFromHighDirect { offset }
                    if offset.absolute_addr() == gbf_hw::lcd::LY_REG =>
                {
                    ly_read_order = Some(item.order());
                }
                _ => {}
            }
        }
        for branch in section.branches() {
            if branch.data.target == bootstrap {
                call_order = Some(branch.order());
            }
        }
        let call_order = call_order.expect("boot calls video bootstrap");
        assert!(ly_read_order < Some(call_order));
        assert!(lcdc_orders.first().copied() < Some(call_order));
        assert!(Some(call_order) < lcdc_orders.last().copied());
    }
}
