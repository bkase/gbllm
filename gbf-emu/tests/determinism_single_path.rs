mod common;

use gbf_emu::{ClockCycles, CycleBudget, DeterminismPolicy, Emulator, EmulatorConfig};

#[test]
fn determinism_single_path() {
    fn run(
        rom: &[u8],
    ) -> (
        Vec<u8>,
        gbf_emu::Regs,
        Vec<u8>,
        Vec<gbf_emu::NormalizedTraceEvent>,
    ) {
        let mut emu = Emulator::load_rom(
            rom,
            EmulatorConfig {
                policy: DeterminismPolicy::default(),
                ..EmulatorConfig::default()
            },
        )
        .expect("ROM loads");
        let _ = emu
            .run_for(CycleBudget::Clock(ClockCycles(1024)))
            .expect("run succeeds");
        let framebuffer = emu.framebuffer().as_bytes().to_vec();
        let regs = emu.regs();
        let memory = emu.peek_range(0xC000, 32).expect("WRAM is peekable");
        let trace = emu.drain_trace();
        (framebuffer, regs, memory, trace)
    }

    let rom = common::tiny_rom();
    assert_eq!(run(&rom), run(&rom));
}
