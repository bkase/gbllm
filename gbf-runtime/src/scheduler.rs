//! Cooperative Bank0 scheduler and TIMA-backed yield-deadline helpers.

use gbf_abi::FaultCode;
use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, Cond, HighDirectOffset, Instr, Reg8};
use gbf_asm::section::{Section, SectionPrivilege, SectionRole, SymbolicBranch, YieldKind};
use gbf_asm::symbols::SymbolName;
use gbf_hw::interrupts::TacClockSelect;
use serde::{Deserialize, Serialize};
use static_assertions::const_assert_eq;

use crate::SECTION_ID_SCHEDULER;

pub const HRAM_ADDR_YIELD_REQUESTED: u16 = 0xFF84;
pub const HRAM_LDH_YIELD_REQUESTED: u8 = 0x84;
pub const HRAM_ADDR_FRAME_COUNT: u16 = 0xFF85;
pub const HRAM_LDH_FRAME_COUNT: u8 = 0x85;
pub const HRAM_ADDR_PREV_CHECKPOINT_LO: u16 = 0xFF86;
pub const HRAM_LDH_PREV_CHECKPOINT_LO: u8 = 0x86;
pub const HRAM_FAST_FLAGS_END_EXCLUSIVE: u16 = 0xFF88;

const_assert_eq!(
    HRAM_ADDR_YIELD_REQUESTED,
    crate::banking::HRAM_BANKING_SHADOW_END_EXCLUSIVE
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimerDeadline {
    pub tac_clock_select: TacClockSelect,
    pub tma: u8,
    pub tima_preload: u8,
    pub requested_m_cycles: u16,
    pub actual_m_cycles: u16,
    pub max_jitter_m_cycles: u8,
}

impl TimerDeadline {
    #[must_use]
    pub const fn bring_up() -> Self {
        Self {
            tac_clock_select: TacClockSelect::Hz4096,
            tma: 0,
            tima_preload: 254,
            requested_m_cycles: 500,
            actual_m_cycles: 512,
            max_jitter_m_cycles: 12,
        }
    }

    #[must_use]
    pub const fn tac_value(self) -> u8 {
        gbf_hw::interrupts::TAC_ENABLE_BIT | (self.tac_clock_select as u8)
    }

    #[must_use]
    pub const fn is_representable(self) -> bool {
        self.actual_m_cycles >= self.requested_m_cycles
            && self.max_jitter_m_cycles as u16 <= self.actual_m_cycles
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchedulerPolicy {
    pub frame_budget_m_cycles: u32,
    pub hard_ui_reserve: u32,
    pub soft_ui_reserve: u32,
    pub video_commit_margin: u32,
    pub max_slice_m_cycles: u32,
    pub adaptive_headroom: u16,
    pub timer_deadline: TimerDeadline,
    pub max_interrupt_entry_latency_m_cycles: u16,
    pub max_interrupt_total_occupancy_m_cycles: u16,
    pub soft_deadline_margin: u32,
    pub max_safe_point_gap_m_cycles: u16,
    pub initial_livelock_threshold_frames: u16,
    pub default_yield_kind: YieldKind,
}

impl SchedulerPolicy {
    #[must_use]
    pub const fn bring_up() -> Self {
        Self {
            frame_budget_m_cycles: gbf_hw::timing::FRAME_M_CYCLES,
            hard_ui_reserve: 3_000,
            soft_ui_reserve: 512,
            video_commit_margin: 256,
            max_slice_m_cycles: 14_000,
            adaptive_headroom: 256,
            timer_deadline: TimerDeadline::bring_up(),
            max_interrupt_entry_latency_m_cycles: 40,
            max_interrupt_total_occupancy_m_cycles: 180,
            soft_deadline_margin: 128,
            max_safe_point_gap_m_cycles: 400,
            initial_livelock_threshold_frames: 120,
            default_yield_kind: YieldKind::Cooperative,
        }
    }

    #[must_use]
    pub const fn fits_frame(self) -> bool {
        self.hard_ui_reserve
            + self.video_commit_margin
            + self.max_slice_m_cycles
            + self.soft_deadline_margin
            <= self.frame_budget_m_cycles
    }

    #[must_use]
    pub const fn safe_point_gap_within_deadline(self) -> bool {
        self.max_safe_point_gap_m_cycles
            <= self.timer_deadline.actual_m_cycles - self.timer_deadline.max_jitter_m_cycles as u16
    }
}

pub fn build_scheduler_section() -> Section {
    let policy = SchedulerPolicy::bring_up();
    let mut builder = Builder::new_with_id(
        SECTION_ID_SCHEDULER,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("scheduler", "section").expect("static symbol"),
    )
    .with_section_privilege(SectionPrivilege::privileged());
    builder.label(SymbolName::runtime("scheduler", "main_loop").expect("static symbol"));
    builder.branch(SymbolicBranch::call(
        SymbolName::runtime("joypad", "read").expect("static symbol"),
        None,
    ));
    builder.branch(SymbolicBranch::call(
        SymbolName::runtime("keyboard", "step").expect("static symbol"),
        None,
    ));
    emit_arm_tima(&mut builder, policy.timer_deadline);
    emit_idle_until_frame(&mut builder);
    builder.branch(SymbolicBranch::jump(
        SymbolName::runtime("scheduler", "main_loop").expect("static symbol"),
        None,
    ));
    builder.finish().with_size_hint_bytes(640)
}

pub fn emit_yield_check(b: &mut Builder, _kind: YieldKind, continue_label: SymbolName) {
    b.emit(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(HRAM_LDH_YIELD_REQUESTED),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    b.branch(SymbolicBranch::jump(continue_label, Some(Cond::Z)));
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(HRAM_LDH_YIELD_REQUESTED),
    });
}

pub fn emit_arm_tima(b: &mut Builder, deadline: TimerDeadline) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: deadline.tma,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::interrupts::TMA_REGISTER),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: deadline.tima_preload,
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::interrupts::TIMA_REGISTER),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: deadline.tac_value(),
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: high(gbf_hw::interrupts::TAC_REGISTER),
    });
}

pub fn emit_idle_until_frame(b: &mut Builder) {
    b.emit(Instr::Ei);
    b.emit(Instr::Halt);
}

#[must_use]
pub const fn liveness_fault(no_progress_frames: u16, threshold: u16) -> Option<FaultCode> {
    if threshold != 0 && no_progress_frames >= threshold {
        Some(FaultCode::LivenessTimeout)
    } else {
        None
    }
}

#[must_use]
pub const fn repeated_checkpoint_fault(
    progress_epoch_advanced: bool,
    previous_checkpoint: u16,
    current_checkpoint: u16,
) -> Option<FaultCode> {
    if !progress_epoch_advanced && previous_checkpoint == current_checkpoint {
        Some(FaultCode::RepeatedCheckpointNoProgress)
    } else {
        None
    }
}

fn high(addr: u16) -> HighDirectOffset {
    HighDirectOffset::new((addr & 0x00FF) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section_effect_kinds;
    use gbf_asm::effect::InterruptControlOp;
    use gbf_asm::effect::MachineEffectKind;
    use gbf_asm::section::BranchKind;

    #[test]
    fn yield_check_emits_expected_sequence() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "yield").unwrap(),
        );
        let continue_label = SymbolName::runtime("test", "continue").unwrap();
        emit_yield_check(&mut builder, YieldKind::Cooperative, continue_label.clone());
        builder.label(continue_label.clone());
        let section = builder.finish();
        assert!(matches!(
            section.instrs()[0].data,
            Instr::LdAFromHighDirect { offset } if offset.get() == HRAM_LDH_YIELD_REQUESTED
        ));
        assert_eq!(section.branches().len(), 1);
        assert_eq!(section.branches()[0].data.kind, BranchKind::Jump);
        assert_eq!(section.branches()[0].data.cond, Some(Cond::Z));
        assert_eq!(section.branches()[0].data.target, continue_label);
        assert!(section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdHighDirectFromA { offset } if offset.get() == HRAM_LDH_YIELD_REQUESTED)
        }));
        assert!(
            !section
                .instrs()
                .iter()
                .any(|item| matches!(item.data, Instr::Ret { .. }))
        );
    }

    #[test]
    fn tima_deadline_is_representable() {
        assert!(TimerDeadline::bring_up().is_representable());
    }

    #[test]
    fn max_safe_point_gap_within_deadline() {
        assert!(SchedulerPolicy::bring_up().safe_point_gap_within_deadline());
    }

    #[test]
    fn halt_invariant() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "halt").unwrap(),
        )
        .with_section_privilege(SectionPrivilege::privileged());
        emit_idle_until_frame(&mut builder);
        let instrs: Vec<_> = builder.finish().instrs().iter().map(|i| i.data).collect();
        for window in instrs.windows(2) {
            if window[1] == Instr::Halt {
                assert_eq!(window[0], Instr::Ei);
            }
        }
    }

    #[test]
    fn livelock_detection() {
        assert_eq!(liveness_fault(5, 5), Some(FaultCode::LivenessTimeout));
        assert_eq!(liveness_fault(4, 5), None);
        assert_eq!(liveness_fault(u16::MAX, 0), None);
    }

    #[test]
    fn repeated_checkpoint_no_progress() {
        assert_eq!(
            repeated_checkpoint_fault(false, 7, 7),
            Some(FaultCode::RepeatedCheckpointNoProgress)
        );
        assert_eq!(repeated_checkpoint_fault(true, 7, 7), None);
        assert_eq!(repeated_checkpoint_fault(false, 7, 8), None);
    }

    #[test]
    fn interrupt_entry_latency() {
        assert!(SchedulerPolicy::bring_up().max_interrupt_entry_latency_m_cycles >= 31);
    }

    #[test]
    fn default_policy_fits_bring_up() {
        assert!(SchedulerPolicy::bring_up().fits_frame());
    }

    #[test]
    fn yield_round_trip() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "roundtrip").unwrap(),
        );
        let continue_label = SymbolName::runtime("test", "roundtrip_continue").unwrap();
        emit_yield_check(&mut builder, YieldKind::Cooperative, continue_label.clone());
        builder.emit(Instr::Ret { cond: None });
        builder.label(continue_label);
        let section = builder.finish();
        assert_eq!(section.branches().len(), 1);
        assert!(
            section
                .instrs()
                .iter()
                .any(|item| matches!(item.data, Instr::LdHighDirectFromA { .. }))
        );
    }

    #[test]
    fn scheduler_emits_timer_io_and_halt() {
        let section = build_scheduler_section();
        let effects = section_effect_kinds(&section);
        assert!(effects.contains(&MachineEffectKind::StoreToIo));
        assert!(effects.contains(&MachineEffectKind::InterruptControl));
        assert!(
            section
                .instrs()
                .iter()
                .any(|item| { matches!(item.data, Instr::Ei | Instr::Halt) })
        );
    }

    #[test]
    fn idle_uses_interrupt_control_ops() {
        assert_eq!(
            gbf_asm::effect::classify_effect(&Instr::Halt),
            gbf_asm::effect::MachineEffect::InterruptControl(InterruptControlOp::Halt)
        );
    }
}
