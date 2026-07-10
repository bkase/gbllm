//! FLASHABLE self-booting subword MoE cartridge gate (env-gated on the bridged
//! dir, like the other real-student MoE gates).
//!
//! Proves the BAKED-prompt demo ROM boots and generates with **NO external
//! poke**: the emitted cartridge bytes are loaded into the emulator, nothing is
//! poked, and the run is driven to `gen_done`. The on-device transcript is then
//! asserted byte-identical to the host mirror render of
//! `subword_host_generate` over the SAME baked prompt + seed, and to
//! `BpeModel::decode` of the generated ids mapped to tiles.
//!
//! Set `MOE_PARITY_DIR` (or `MOE_INT_DIR`) to a dir with `ckpt/` and
//! `tokenizer/gbllm_bpe.v2.json`. `MOE_DEMO_NGEN` overrides the token count
//! (default 12; keep modest — the real router is minutes/token).

use std::path::PathBuf;

use gbf_bench::shell::framebuffer_to_pgm;
use gbf_bench::stateful::load_state_checkpoint;
use gbf_bench::subword_demo::{
    decode_ids, expected_subword_transcript_bg, subword_font_tiles, subword_host_generate,
};
use gbf_emu::{
    BootMode, ClockCycles, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator,
    RunOutcome, TraceDropPolicy,
};
use gbf_kernel::asm_impl_shell::{
    BG_MAP_BASE, BG_MAP_STRIDE, TRANSCRIPT_COLS, TRANSCRIPT_ROWS, build_state_moe_demo_rom_baked,
};
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LogitPaging};

const TOP_K: u8 = 8;
const TEMPERATURE: f64 = 0.8;
const RNG_SEED: u16 = 0x5EED;
const PROMPT: &str = "Once upon a time";

fn moe_parity_dir() -> Option<PathBuf> {
    std::env::var("MOE_PARITY_DIR")
        .or_else(|_| std::env::var("MOE_INT_DIR"))
        .ok()
        .map(PathBuf::from)
}

#[test]
#[ignore = "requires MOE_PARITY_DIR pointing at a real bridged MoE student + BPE artifact"]
fn baked_cartridge_self_boots_and_generates_byte_identically_to_host() {
    let Some(root) = moe_parity_dir() else {
        eprintln!("MOE_PARITY_DIR / MOE_INT_DIR unset; skipping");
        return;
    };
    let n_gen: u8 = std::env::var("MOE_DEMO_NGEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let ckpt = root.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)
        .unwrap_or_else(|e| panic!("load real MoE student at {}: {e}", ckpt.display()));
    let topo = bundle.topology;
    assert!(topo.is_moe(), "expected MoE (n_experts={})", topo.n_experts);
    assert_eq!(topo.logit_paging, LogitPaging::Paged, "vocab-1024 is Paged");
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .unwrap_or_else(|e| panic!("lower real MoE student: {e}"));

    let bpe_path = root.join("tokenizer/gbllm_bpe.v2.json");
    let bpe_text = std::fs::read_to_string(&bpe_path)
        .unwrap_or_else(|e| panic!("read BPE artifact {}: {e}", bpe_path.display()));
    let bpe = gbf_data::bpe::BpeModel::from_json(&bpe_text)
        .unwrap_or_else(|e| panic!("parse BPE artifact: {e}"));
    assert_eq!(bpe.vocab_size(), topo.vocab, "BPE vocab matches the model");
    let id_bytes: Vec<Vec<u8>> = (0..topo.vocab)
        .map(|id| bpe.id_bytes(id as u16).expect("id in vocab").to_vec())
        .collect();

    let font = subword_font_tiles();
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(TOP_K, step, TEMPERATURE).expect("valid sampler");

    let prompt_ids = bpe.encode(PROMPT);
    assert!(!prompt_ids.is_empty());

    // Build the SELF-BOOTING baked cartridge (prompt + seed baked into ROM).
    let rom = build_state_moe_demo_rom_baked(
        &lowered,
        &cfg,
        n_gen,
        &font,
        &id_bytes,
        &prompt_ids,
        RNG_SEED,
    )
    .expect("baked subword MoE cartridge builds within budget");
    println!(
        "baked cartridge: banks={} driver={} ui={} id_bytes_stride={} table={}",
        rom.bank_count,
        rom.driver_bytes,
        rom.ui_bank_bytes,
        rom.id_bytes_geom.stride,
        rom.table_bytes,
    );
    println!("  prompt = {PROMPT:?}  ids = {prompt_ids:?}  seed = {RNG_SEED:#06x}");

    // Host mirror over the SAME baked prompt + seed (the parity oracle).
    let host_seq = subword_host_generate(&lowered, &cfg, &id_bytes, &prompt_ids, RNG_SEED, n_gen);

    // Boot the emitted cartridge bytes; poke NOTHING.
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .expect("load baked cartridge");

    let frame_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    // Per-token budget: MoE MAC budget ×4, matching the poked session runner.
    let macs = lowered.topology.macs_per_token();
    let floor = DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000).0;
    let per_mac = 4096u64;
    let base = floor.max(macs.saturating_mul(per_mac));
    let token_budget = CycleBudget::Clock(ClockCycles(base.saturating_mul(4)));

    // `step` first so a boundary PC we are already sitting on does not
    // immediately re-trap (mirrors the poked session runner).
    let run_to = |emu: &mut Emulator, pc: u16, phase: &str, budget: CycleBudget, step: bool| {
        if step {
            emu.step().expect("advance past boundary");
        }
        match emu.run_fast_until_pc(pc, budget).expect("emu runs") {
            RunOutcome::TrapHit { .. } => {}
            other => panic!("did not reach {phase} at {pc:#06x}: {other:?}"),
        }
    };

    // NO poke: the baked boot prologue sets go/plen/seed itself. First reach the
    // idle head (boot), then the run auto-starts. Drive through each warmup +
    // generation boundary.
    run_to(&mut emu, rom.idle_pc, "idle", frame_budget, false);
    for _ in 0..prompt_ids.len() {
        run_to(
            &mut emu,
            rom.warm_boundary_pc,
            "warm boundary",
            token_budget,
            true,
        );
    }
    for _ in 0..host_seq.len() {
        run_to(
            &mut emu,
            rom.token_boundary_pc,
            "token boundary",
            token_budget,
            true,
        );
    }
    run_to(
        &mut emu,
        rom.gen_done_pc,
        "generation done",
        token_budget,
        true,
    );

    // Read the transcript BG region and compare to the host mirror render.
    let cols = usize::from(TRANSCRIPT_COLS);
    let mut bg = Vec::new();
    for row in 0..TRANSCRIPT_ROWS {
        let addr = BG_MAP_BASE + u16::from(row) * BG_MAP_STRIDE;
        bg.extend_from_slice(&emu.peek_range(addr, cols).expect("peek BG row"));
    }
    let expected = expected_subword_transcript_bg(&host_seq, &id_bytes);
    let first_mismatch = bg
        .iter()
        .zip(expected.iter())
        .enumerate()
        .find(|(_, (a, e))| a != e)
        .map(|(i, (&a, &e))| (i, e, a));

    let host_text = bpe.decode(&host_seq);
    let render_text = decode_ids(&host_seq, &id_bytes);
    println!("baked cartridge generated (NO poke):");
    println!("  ids    = {host_seq:?}");
    println!("  decode = {host_text:?}");
    println!("  render = {render_text:?}");

    // Screenshot the final screen.
    let pgm = framebuffer_to_pgm(&emu.framebuffer());
    let pgm_path = "/private/tmp/claude-501/moe_demo_screen.pgm";
    std::fs::write(pgm_path, &pgm).expect("write pgm");
    println!("  screenshot -> {pgm_path}");

    assert!(
        first_mismatch.is_none() && bg.len() == expected.len(),
        "on-device transcript != host render; first mismatch {first_mismatch:?}",
    );
    assert_eq!(
        render_text, host_text,
        "rendered transcript == BpeModel::decode(token_ids)"
    );
    assert!(
        !host_seq.is_empty(),
        "cartridge generated tokens with no poke"
    );
}
