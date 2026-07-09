//! Capture framebuffers *during* on-device token compute, so the inference
//! animation can be seen (montage the PGMs into a grid / GIF).
//!
//! Boots the deployed d192 shell ROM, types a short prompt, presses START, then
//! grabs a framebuffer every ~fixed cycle slice across the warmup+generation
//! window. Writes `frame_NNN.pgm` (160x144, P5) to the output dir.
//!
//! Usage: `cargo run --release -p gbf-bench --bin anim-preview [-- out_dir n_frames]`

use gbf_bench::d192_real::D192_REAL_EXPORT_DIR;
use gbf_bench::shell::{
    SHELL_TEMPERATURE, SHELL_TOP_K, char_to_id, framebuffer_to_pgm, shell_font_tiles, typing_script,
};
use gbf_bench::stateful::load_state_checkpoint;
use gbf_emu::{
    BootMode, ClockCycles, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator,
    JoypadFrame, RunOutcome, TraceDropPolicy,
};
use gbf_hw::joypad::Button;
use gbf_kernel::asm_impl_shell::build_state_shell_rom;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::IntStateLoweredModel;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let out_dir = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| "/private/tmp/claude-501/anim-preview".to_string()),
    );
    let n_frames: usize = args.get(2).map_or(Ok(36), |s| s.parse())?;
    std::fs::create_dir_all(&out_dir)?;

    let bundle = load_state_checkpoint(&PathBuf::from(D192_REAL_EXPORT_DIR))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(SHELL_TOP_K, step, SHELL_TEMPERATURE)?;
    let rom = build_state_shell_rom(&lowered, &cfg, 8, &shell_font_tiles())?;

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::DropNewest)
        .load_rom(&rom.rom)?;

    let frame_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    let run_to = |emu: &mut Emulator, pc: u16| -> Result<(), Box<dyn std::error::Error>> {
        match emu.run_fast_until_pc(pc, frame_budget)? {
            RunOutcome::TrapHit { .. } => Ok(()),
            other => Err(format!("did not reach {pc:#06x}: {other:?}").into()),
        }
    };

    // Boot to idle, type the prompt, submit.
    run_to(&mut emu, rom.idle_pc)?;
    let prompt_ids: Vec<u8> = "the".chars().map(|c| char_to_id(c).unwrap()).collect();
    for frame in typing_script(&prompt_ids) {
        emu.set_joypad(frame);
        // step past the parked idle PC so this joypad frame is actually polled,
        // then run to the next idle-loop head (mirrors bench `step_run_to`).
        emu.step()?;
        run_to(&mut emu, rom.idle_pc)?;
    }
    // Submit: hold START until the ROM leaves idle and reaches the first warmup
    // boundary (proves generation actually started before we sample compute).
    emu.set_joypad(JoypadFrame::pressed(Button::Start));
    let token_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000));
    emu.step()?; // leave the parked idle PC so START is polled
    match emu.run_fast_until_pc(rom.warm_boundary_pc, token_budget)? {
        RunOutcome::TrapHit { .. } => {}
        other => {
            return Err(format!("never reached warmup (prompt didn't submit?): {other:?}").into());
        }
    }
    emu.set_joypad(JoypadFrame::default());
    eprintln!(
        "reached warmup boundary (in generation now); fc so far = {}",
        emu.peek(rom.layout.shell.as_ref().unwrap().prompt + 0x2A)
            .unwrap_or(0xEE)
    );

    // Capture across the remaining warmup + first-token compute window.
    let slice = CycleBudget::Clock(ClockCycles(2_600_000));
    for i in 0..n_frames {
        emu.run_fast_for(slice)?;
        let scx = emu.bus_read(0xFF43).unwrap_or(0xEE);
        let scy = emu.bus_read(0xFF42).unwrap_or(0xEE);
        let pc = emu.regs().pc;
        let fc_addr = rom.layout.shell.as_ref().unwrap().prompt + 0x2A;
        let fc = emu.peek(fc_addr).unwrap_or(0xEE);
        eprintln!("frame {i:03}: pc={pc:#06x} scx={scx} scy={scy} fc[{fc_addr:#06x}]={fc}");
        let pgm = framebuffer_to_pgm(&emu.framebuffer());
        std::fs::write(out_dir.join(format!("frame_{i:03}.pgm")), pgm)?;
    }
    println!("wrote {n_frames} frames to {}", out_dir.display());
    Ok(())
}
