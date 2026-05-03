mod common;

use gbf_emu::{
    AddressRange, BootMode, ClockCycles, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy,
    EmuError, Emulator, Predicate, RunOutcome, TraceOrigin, TrapAction, TrapKind,
};

fn emu_for(program: &[u8]) -> Emulator {
    Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .load_rom(&common::rom(program, 0x00, 0x00))
        .expect("ROM loads")
}

#[test]
fn fast_run_until_pc_reaches_target_without_full_step_trace() {
    let mut emu = emu_for(&[0x00]);

    let outcome = emu
        .run_fast_until_pc(0x0150, CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES))
        .expect("fast run succeeds");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit {
            trap_id: gbf_emu::BreakpointId::RUN_UNTIL_PC,
            kind: TrapKind::Pc { addr: 0x0150 },
            ..
        }
    ));
}

#[test]
fn fast_run_honors_pc_traps() {
    let mut emu = emu_for(&[0x00]);
    let id = emu
        .traps()
        .add_pc(0x0150, Predicate::Always, TrapAction::HaltAndReport);

    let outcome = emu
        .run_fast_for(CycleBudget::Clock(ClockCycles(64)))
        .expect("fast run succeeds");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit {
            trap_id,
            kind: TrapKind::Pc { addr: 0x0150 },
            ..
        } if trap_id == id
    ));
}

#[test]
fn fast_run_rejects_memory_traps_without_advancing() {
    let mut emu = emu_for(&[0xEA, 0x00, 0xC0, 0x00]);
    let range = AddressRange::new(0xC000, 0xC000).expect("valid range");
    let id = emu
        .traps()
        .add_mem_write(range, Predicate::Always, TrapAction::HaltAndReport);
    let start = emu.clock_count();

    let err = emu
        .run_fast_for(CycleBudget::Clock(ClockCycles(128)))
        .expect_err("fast run rejects memory traps");

    assert!(matches!(
        err,
        EmuError::FastRunBlockedByMemoryTraps {
            memory_trap_count: 1
        }
    ));
    assert_eq!(emu.clock_count(), start);
    assert!(emu.drain_trace().is_empty());

    let outcome = emu
        .run_for(CycleBudget::Clock(ClockCycles(128)))
        .expect("caller can explicitly choose slow runner");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit {
            trap_id,
            kind: TrapKind::MemWrite { range: hit_range },
            ..
        } if trap_id == id && hit_range == range
    ));
    assert!(emu.drain_trace().iter().any(|event| matches!(
        event,
        gbf_emu::NormalizedTraceEvent::MemoryWrite {
            addr: 0xC000,
            value: 0x01,
            origin: TraceOrigin::GuestCpu,
            ..
        }
    )));
}

#[test]
fn fast_run_suppresses_guest_memory_trace_when_no_memory_traps_are_present() {
    let mut emu = emu_for(&[0xEA, 0x00, 0xC0, 0x00]);

    let outcome = emu
        .run_fast_for(CycleBudget::Clock(ClockCycles(128)))
        .expect("fast run succeeds");

    assert!(matches!(outcome, RunOutcome::BudgetElapsed { .. }));
    assert!(emu.drain_trace().is_empty());
    assert_eq!(emu.peek(0xC000).expect("WRAM peek"), 0x01);
}

#[test]
fn fast_frame_runs_one_dmg_frame_budget() {
    let mut emu = emu_for(&[0x00]);
    let start = emu.clock_count();

    let outcome = emu.run_fast_frame().expect("fast frame succeeds");

    assert!(matches!(outcome, RunOutcome::BudgetElapsed { .. }));
    assert!(emu.clock_count() >= ClockCycles(start.0 + DMG_FRAME_CLOCK_CYCLES.0));
}
