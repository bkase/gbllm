mod common;

use std::cell::Cell;
use std::rc::Rc;

use gbf_emu::{
    AddressRange, ClockCycles, CycleBudget, Emulator, EmulatorConfig, Predicate, RunOutcome,
    TrapAction, TrapKind, TrapPredicateError,
};

#[test]
fn pc_breakpoint_fires_before_instruction() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let pc = emu.regs().pc;
    let id = emu
        .traps()
        .add_pc(pc, Predicate::Always, TrapAction::HaltAndReport);

    assert_eq!(
        emu.step().expect("step hits trap"),
        gbf_emu::StepOutcome::TrapHit {
            trap_id: id,
            kind: TrapKind::Pc { addr: pc },
            cycles: ClockCycles(0)
        }
    );
    assert_eq!(emu.regs().pc, pc);
}

#[test]
fn pc_continue_trap_executes_instruction() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let pc = emu.regs().pc;
    emu.traps()
        .add_pc(pc, Predicate::Always, TrapAction::Continue);

    let outcome = emu.step().expect("step succeeds");

    assert!(matches!(
        outcome,
        gbf_emu::StepOutcome::Stepped {
            cycles: ClockCycles(16)
        }
    ));
    assert_eq!(emu.regs().pc, 0x0150);
    assert!(matches!(
        emu.drain_trace().as_slice(),
        [gbf_emu::NormalizedTraceEvent::TrapHit { .. }]
    ));
}

#[test]
fn pc_continue_does_not_hide_later_pc_halt_trap() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let pc = emu.regs().pc;
    emu.traps()
        .add_pc(pc, Predicate::Always, TrapAction::Continue);
    let halt_id = emu
        .traps()
        .add_pc(pc, Predicate::Always, TrapAction::HaltAndReport);

    let outcome = emu.step().expect("step hits second PC trap");

    assert_eq!(
        outcome,
        gbf_emu::StepOutcome::TrapHit {
            trap_id: halt_id,
            kind: TrapKind::Pc { addr: pc },
            cycles: ClockCycles(0)
        }
    );
    assert_eq!(emu.regs().pc, pc);
    assert!(
        emu.drain_trace()
            .iter()
            .any(|event| matches!(event, gbf_emu::NormalizedTraceEvent::TrapHit { .. }))
    );
}

#[test]
fn mem_watchpoint_write_reports_first_matching_access_after_step() {
    let rom = common::rom(&[0x3E, 0x42, 0xEA, 0x00, 0xC0, 0x76], 0x00, 0x00);
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
    let id = emu.traps().add_mem_write(
        AddressRange::new(0xC000, 0xC000).unwrap(),
        Predicate::Always,
        TrapAction::HaltAndReport,
    );

    let outcome = emu
        .run_for(CycleBudget::Clock(ClockCycles(128)))
        .expect("run succeeds");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit {
            trap_id,
            kind: TrapKind::MemWrite { .. },
            ..
        } if trap_id == id
    ));
    assert_eq!(emu.peek(0xC000), Ok(0x42));
}

#[test]
fn mem_read_and_rw_traps_include_instruction_fetch_and_post_instruction_state() {
    let rom = common::rom(&[0x00, 0x76], 0x00, 0x00);
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
    let id = emu.traps().add_mem_read(
        AddressRange::new(0x0150, 0x0150).unwrap(),
        Predicate::Closure(Box::new(|ctx| {
            assert_eq!(ctx.access.expect("memory access").addr, 0x0150);
            assert_eq!(ctx.regs.pc, 0x0151);
            Ok(true)
        })),
        TrapAction::HaltAndReport,
    );

    assert!(matches!(
        emu.run_for(CycleBudget::Clock(ClockCycles(64)))
            .expect("run succeeds"),
        RunOutcome::TrapHit {
            trap_id,
            kind: TrapKind::MemRead { .. },
            ..
        } if trap_id == id
    ));

    let mut emu = Emulator::load_rom(
        &common::rom(&[0x3E, 0x22, 0xEA, 0x00, 0xC0, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let id = emu.traps().add_mem_rw(
        AddressRange::new(0xC000, 0xC000).unwrap(),
        Predicate::Closure(Box::new(|ctx| {
            assert_eq!(ctx.view.peek(0xC000), Ok(0x22));
            Ok(true)
        })),
        TrapAction::HaltAndReport,
    );

    assert!(matches!(
        emu.run_for(CycleBudget::Clock(ClockCycles(128)))
            .expect("run succeeds"),
        RunOutcome::TrapHit {
            trap_id,
            kind: TrapKind::MemRw { .. },
            ..
        } if trap_id == id
    ));
}

#[test]
fn predicate_errors_surface_typed_and_closure_runs_once() {
    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let pc = emu.regs().pc;
    emu.traps().add_pc(
        pc,
        Predicate::Source("pc == 0x0100".to_owned()),
        TrapAction::HaltAndReport,
    );

    assert_eq!(
        emu.step(),
        Err(gbf_emu::EmuError::TrapPredicate(
            TrapPredicateError::SourceRequiresEvaluator
        ))
    );

    let mut emu = Emulator::load_rom(
        &common::rom(&[0x00, 0x76], 0x00, 0x00),
        EmulatorConfig::default(),
    )
    .expect("ROM loads");
    let calls = Rc::new(Cell::new(0));
    let seen = Rc::clone(&calls);
    let pc = emu.regs().pc;
    emu.traps().add_pc(
        pc,
        Predicate::Closure(Box::new(move |_| {
            seen.set(seen.get() + 1);
            Err(TrapPredicateError::PredicateFailed {
                reason: "boom".to_owned(),
            })
        })),
        TrapAction::HaltAndReport,
    );

    assert_eq!(
        emu.step(),
        Err(gbf_emu::EmuError::TrapPredicate(
            TrapPredicateError::PredicateFailed {
                reason: "boom".to_owned()
            }
        ))
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn memory_continue_trap_does_not_hide_later_halt_trap() {
    let rom = common::rom(
        &[0x3E, 0x42, 0xEA, 0x00, 0xC0, 0xEA, 0x01, 0xC0, 0x76],
        0x00,
        0x00,
    );
    let mut emu = Emulator::load_rom(&rom, EmulatorConfig::default()).expect("ROM loads");
    emu.traps().add_mem_write(
        AddressRange::new(0xC000, 0xC000).unwrap(),
        Predicate::Always,
        TrapAction::Continue,
    );
    let halt_id = emu.traps().add_mem_write(
        AddressRange::new(0xC001, 0xC001).unwrap(),
        Predicate::Always,
        TrapAction::HaltAndReport,
    );

    let outcome = emu
        .run_for(CycleBudget::Clock(ClockCycles(256)))
        .expect("run succeeds");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit { trap_id, .. } if trap_id == halt_id
    ));
    assert!(
        emu.drain_trace()
            .iter()
            .any(|event| matches!(event, gbf_emu::NormalizedTraceEvent::TrapHit { .. }))
    );
}

#[test]
fn remove_list_and_persistable_specs_round_trip() {
    let mut traps = gbf_emu::TrapDispatcher::new();
    let id = traps.add_pc(
        0x0150,
        Predicate::Source("pc == 0x0150".to_owned()),
        TrapAction::Continue,
    );
    let entries = traps.list().collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, id);
    assert_eq!(entries[0].persistable_predicate, Some("pc == 0x0150"));

    let specs = traps.export_persistable_specs().expect("spec exports");
    let encoded = serde_json::to_string(&specs).expect("spec serializes");
    let decoded: Vec<gbf_emu::TrapSpec> =
        serde_json::from_str(&encoded).expect("spec deserializes");
    assert_eq!(decoded, specs);

    let removed = traps.remove_entry(id).expect("entry removed");
    assert_eq!(removed.id, id);
    assert!(!traps.remove(id));
}
