//! Per-phase cycle profiler for one on-device token of the real d192 state model.
//!
//! Single-steps the one-token state ROM from `token_start_pc` to
//! `token_end_pc`, attributing each instruction's M-cycles to a PC bucket.
//! `pc >= 0x4000` is the switched weight-code banks (matvec + per-row
//! epilogue); `pc < 0x4000` is the fixed driver bank (norm, GELU/dequant, state
//! decay, out-projection quantize, dispatch, sampling, head). It also emits a
//! per-routine breakdown (via the label map) and a 256-byte page histogram of
//! the driver bank so the hottest driver routines are visible. This tells us
//! where the M-cycles/token actually go before we optimize. Read-only; changes
//! no numerics.
//!
//! Usage: `cargo run --release -p gbf-bench --bin cycle-profile [-- export_dir]`

use gbf_bench::d192_real::D192_REAL_EXPORT_DIR;
use gbf_bench::stateful::load_state_checkpoint;
use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    StepOutcome, TraceDropPolicy,
};
use gbf_kernel::asm_impl_state::{S_INPUT_ADDR, WeightLowering, build_state_one_token_rom_debug};
use gbf_kernel::state_model_ref::IntStateLoweredModel;
use std::path::PathBuf;

const DMG_M_CYCLES_PER_SECOND: u64 = 1_048_576;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let export_dir = args
        .get(1)
        .map_or_else(|| D192_REAL_EXPORT_DIR.to_owned(), Clone::clone);

    let bundle = load_state_checkpoint(&PathBuf::from(&export_dir))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;
    let (rom, labels) = build_state_one_token_rom_debug(&lowered, WeightLowering::V3)?;
    // Sorted (addr, name) for nearest-label-below lookups on a driver PC.
    let mut sym: Vec<(u16, String)> = labels
        .into_iter()
        .filter(|(_, addr)| *addr < 0x4000)
        .map(|(name, addr)| (addr, name))
        .collect();
    sym.sort();
    let routine_of = |pc: u16| -> &str {
        match sym.binary_search_by(|(a, _)| a.cmp(&pc)) {
            Ok(i) => &sym[i].1,
            Err(0) => "<pre-driver>",
            Err(i) => &sym[i - 1].1,
        }
    };
    eprintln!(
        "profiling one token: {} MACs, driver {} B, weight-code {} B ({} chunks)",
        lowered.topology.macs_per_token(),
        rom.driver_bytes,
        rom.weight_code_bytes,
        rom.weight_chunk_count,
    );

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::DropNewest)
        .load_rom(&rom.rom)?;
    // Input id 75, zero carried state (the gate's zero-state case).
    emu.poke(S_INPUT_ADDR, 75)?;

    let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000));
    match emu.run_fast_until_pc(rom.token_start_pc, budget)? {
        RunOutcome::TrapHit { .. } => {}
        other => return Err(format!("did not reach token start: {other:?}").into()),
    }

    // Single-step to token end, bucketing clock cycles by PC region.
    let mut matvec_clocks: u64 = 0;
    let mut driver_clocks: u64 = 0;
    let mut driver_pages = [0u64; 64]; // page = pc >> 8 for pc < 0x4000
    let mut by_routine: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut steps: u64 = 0;
    loop {
        let pc = emu.regs().pc;
        if pc == rom.token_end_pc {
            break;
        }
        let out = emu.step()?;
        let c = match out {
            StepOutcome::Stepped { cycles } | StepOutcome::Idle { cycles, .. } => cycles.0,
            StepOutcome::TrapHit { cycles, .. } => cycles.0,
        };
        if pc >= 0x4000 {
            matvec_clocks += c;
        } else {
            driver_clocks += c;
            driver_pages[(pc >> 8) as usize] += c;
            *by_routine.entry(routine_of(pc).to_owned()).or_default() += c;
        }
        steps += 1;
        if steps > 40_000_000 {
            return Err("step cap exceeded — token never ended".into());
        }
    }

    let total_clocks = matvec_clocks + driver_clocks;
    let m = |clk: u64| clk / 4; // 4 T-cycles per M-cycle
    let total_m = m(total_clocks);
    let pct = |clk: u64| 100.0 * clk as f64 / total_clocks.max(1) as f64;

    println!("\n=== one-token cycle profile (real d192, V3) ===");
    println!("instructions retired : {steps}");
    println!(
        "total                : {:>12} M-cycles = {:.3} s/char on DMG",
        total_m,
        total_m as f64 / DMG_M_CYCLES_PER_SECOND as f64
    );
    println!(
        "matvec (pc>=0x4000)  : {:>12} M-cycles  {:5.1}%",
        m(matvec_clocks),
        pct(matvec_clocks)
    );
    println!(
        "driver (pc< 0x4000)  : {:>12} M-cycles  {:5.1}%",
        m(driver_clocks),
        pct(driver_clocks)
    );

    println!("\n--- driver cycles by routine (named label ranges) ---");
    let mut routines: Vec<(String, u64)> = by_routine.into_iter().collect();
    routines.sort_by_key(|&(_, clk)| std::cmp::Reverse(clk));
    for (name, clk) in routines.into_iter().take(20) {
        println!("  {:<28} {:>11} M-cycles  {:5.1}%", name, m(clk), pct(clk));
    }

    println!("\n--- hottest driver pages (256-byte granularity) ---");
    let mut pages: Vec<(usize, u64)> = driver_pages
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, c)| c > 0)
        .collect();
    pages.sort_by_key(|&(_, clk)| std::cmp::Reverse(clk));
    for (page, clk) in pages.into_iter().take(20) {
        println!(
            "  {:#06x}-{:#06x} : {:>11} M-cycles  {:5.1}%  [{}..]",
            page << 8,
            (page << 8) + 0xff,
            m(clk),
            pct(clk),
            routine_of((page << 8) as u16),
        );
    }
    Ok(())
}
