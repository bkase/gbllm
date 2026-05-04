//! Bank0-resident interrupt handler bodies and narrow IF helper.

use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, HighDirectOffset, Instr, Reg8};
use gbf_asm::section::{
    ExecutionContext, InterruptDiscipline, Section, SectionPrivilege, SectionRole, SymbolicBranch,
};
use gbf_asm::symbols::SymbolName;
use gbf_hw::interrupts::InterruptSource;

use crate::SECTION_ID_INTERRUPTS;
use crate::scheduler::{HRAM_LDH_FRAME_COUNT, HRAM_LDH_YIELD_REQUESTED};

pub const INTERRUPT_HANDLERS_BASE_ADDR: u16 = 0x02C0;
pub const INTERRUPT_HANDLER_BYTES: u16 = 0x20;

pub fn build_handlers_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_INTERRUPTS,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("interrupts", "handlers").expect("static symbol"),
    )
    .with_section_privilege(
        SectionPrivilege::interrupt_handler()
            .with_execution_context(ExecutionContext::InterruptHandler)
            .with_interrupt_discipline(InterruptDiscipline::ImeDisabled),
    );

    builder.label(SymbolName::runtime("interrupts", "vblank_handler").expect("static symbol"));
    emit_vblank_handler(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("interrupts", "lcd_stat_handler").expect("static symbol"));
    emit_lcd_stat_handler(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("interrupts", "timer_handler").expect("static symbol"));
    emit_timer_handler(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("interrupts", "serial_handler").expect("static symbol"));
    emit_serial_handler(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("interrupts", "joypad_handler").expect("static symbol"));
    emit_joypad_handler(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.finish().with_size_hint_bytes(220)
}

pub fn emit_vblank_handler(b: &mut Builder) {
    b.emit(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(HRAM_LDH_FRAME_COUNT),
    });
    b.emit(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(HRAM_LDH_FRAME_COUNT),
    });
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("video_commit", "drain_vblank").expect("static symbol"),
        None,
    ));
}

pub fn emit_lcd_stat_handler(b: &mut Builder) {
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("video_commit", "drain_hblank").expect("static symbol"),
        None,
    ));
}

pub fn emit_timer_handler(b: &mut Builder) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 1,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(HRAM_LDH_YIELD_REQUESTED),
    });
}

pub fn emit_serial_handler(_b: &mut Builder) {}

pub fn emit_joypad_handler(_b: &mut Builder) {}

/// Software-only discard of a pending IF bit. Normal ISR entry does not call
/// this; the CPU acknowledges the selected interrupt before handler execution.
pub fn emit_clear_pending_if_bit(b: &mut Builder, source: InterruptSource) {
    let mask = !gbf_hw::interrupts::if_bit(source);
    b.emit(Instr::LdAFromHighDirect {
        offset: if_offset(),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(mask),
    });
    b.emit(Instr::OrA {
        src: AluSrc8::Imm(gbf_hw::interrupts::IE_IF_UNUSED_MASK),
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: if_offset(),
    });
}

#[must_use]
pub fn handler_m_cycles(source: InterruptSource) -> u16 {
    let mut builder = Builder::new(
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("interrupts", "cycles").expect("static symbol"),
    )
    .with_section_privilege(SectionPrivilege::interrupt_handler());
    match source {
        InterruptSource::VBlank => emit_vblank_handler(&mut builder),
        InterruptSource::LcdStat => emit_lcd_stat_handler(&mut builder),
        InterruptSource::Timer => emit_timer_handler(&mut builder),
        InterruptSource::Serial => emit_serial_handler(&mut builder),
        InterruptSource::Joypad => emit_joypad_handler(&mut builder),
    }
    builder
        .finish()
        .instrs()
        .iter()
        .map(|item| u16::from(item.data.cycle_cost().worst_case()))
        .sum()
}

fn if_offset() -> HighDirectOffset {
    HighDirectOffset::new((gbf_hw::interrupts::IF_REGISTER & 0x00FF) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot;
    use crate::scheduler::SchedulerPolicy;
    use gbf_asm::effect::PrivilegeClass;
    use gbf_asm::section::{ExecutionContext, InterruptDiscipline};

    #[test]
    fn isr_stubs_are_isr_marked() {
        let section = boot::build_isr_stubs_section();
        assert_eq!(
            section.privilege().default_privilege,
            PrivilegeClass::InterruptHandler
        );
        assert_eq!(
            section.privilege().execution_context,
            ExecutionContext::InterruptHandler
        );
        assert_eq!(
            section.privilege().interrupt_discipline,
            InterruptDiscipline::ImeDisabled
        );
    }

    #[test]
    fn handlers_do_not_clobber_unrelated_if_bits() {
        let handlers = build_handlers_section();
        assert!(!handlers.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdHighDirectFromA { offset } if offset.absolute_addr() == gbf_hw::interrupts::IF_REGISTER)
        }));

        let mut helper = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "if_clear").unwrap(),
        );
        emit_clear_pending_if_bit(&mut helper, InterruptSource::Timer);
        let helper = helper.finish();
        assert!(
            helper
                .instrs()
                .iter()
                .any(|item| matches!(item.data, Instr::AndA { .. }))
        );
        assert!(
            helper
                .instrs()
                .iter()
                .any(|item| matches!(item.data, Instr::OrA { .. }))
        );
    }

    #[test]
    fn vblank_handler_bumps_frame_count() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "vblank").unwrap(),
        )
        .with_section_privilege(SectionPrivilege::interrupt_handler());
        emit_vblank_handler(&mut builder);
        let section = builder.finish();
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdHighDirectFromA { offset } if offset.get() == HRAM_LDH_FRAME_COUNT)
        }));
    }

    #[test]
    fn timer_handler_sets_yield_requested() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "timer").unwrap(),
        )
        .with_section_privilege(SectionPrivilege::interrupt_handler());
        emit_timer_handler(&mut builder);
        let section = builder.finish();
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdHighDirectFromA { offset } if offset.get() == HRAM_LDH_YIELD_REQUESTED)
        }));
    }

    #[test]
    fn isr_entry_latency_under_policy_bound() {
        assert!(SchedulerPolicy::bring_up().max_interrupt_entry_latency_m_cycles >= 31);
    }

    #[test]
    fn isr_total_occupancy_under_policy_bound() {
        let policy = SchedulerPolicy::bring_up();
        for source in InterruptSource::ALL {
            assert!(handler_m_cycles(source) <= policy.max_interrupt_total_occupancy_m_cycles);
        }
    }
}
