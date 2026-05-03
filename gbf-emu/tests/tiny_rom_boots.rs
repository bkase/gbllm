mod common;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TrapKind,
};

const MAX_TINY_ROM_BOOT_BUDGET: CycleBudget =
    CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(100));

#[test]
fn tiny_rom_boots_to_entry() {
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .load_rom(&common::tiny_rom())
        .expect("tiny ROM loads");

    let outcome = emu
        .run_fast_until_pc(0x0150, MAX_TINY_ROM_BOOT_BUDGET)
        .expect("run does not fault");

    assert!(matches!(
        outcome,
        RunOutcome::TrapHit {
            kind: TrapKind::Pc { addr: 0x0150 },
            ..
        }
    ));
}
