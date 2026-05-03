use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TrapKind,
};

const TINY_ROM: &[u8] = include_bytes!("../tests/fixtures/tiny_rom.gb");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .load_rom(TINY_ROM)?;

    match emu.run_fast_until_pc(
        0x0150,
        CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(100)),
    )? {
        RunOutcome::TrapHit {
            kind: TrapKind::Pc { addr: 0x0150 },
            ..
        } => {
            println!(
                "tiny_rom reached $0150 at {} clock cycles",
                emu.clock_count().0
            );
            Ok(())
        }
        outcome => Err(format!("tiny_rom did not reach $0150: {outcome:?}").into()),
    }
}
