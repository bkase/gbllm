//! V=1024 logit paging (deploy step 2, bd-3vr6s): byte-exact paged
//! head/argmax/sampler on a SYNTHETIC DENSE model (orthogonal to MoE).
//!
//! - `greedy_paged_decode_equals_single_page_decode_small_vocab`: a <=85-vocab
//!   dense model built both `SinglePage` and `Paged` must produce the identical
//!   greedy argmax sequence and stay byte-exact on-device in both modes.
//! - `paged_decode_on_vocab_1024_is_byte_exact`: a synthetic DENSE vocab-1024
//!   `Paged` model runs greedy through `run_state_rom_gate`; the gate compares
//!   the ROM's u16 argmax16, the last logit page, and the finalized heap
//!   byte-exactly against the host.
//! - `topk_heap_sampling_parity_host_vs_golden`: a vocab-1024 `Paged` sampling
//!   ROM's `S_SAMPLED` + heap WRAM equals `decode::sample_topk_from_candidates`
//!   golden for a fixed seed, k in {8, 40}.

use gbf_bench::stateful::run_state_rom_gate;
use gbf_emu::{
    BootMode, ClockCycles, CycleBudget, DeterminismPolicy, Emulator, RunOutcome, TraceDropPolicy,
};
use gbf_kernel::asm_impl_state::{
    S_INPUT_ADDR, S_RNG_ADDR, S_SAMPLED_ADDR, StateWramLayout, build_state_multi_token_sampling_rom,
};
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_from_candidates_trace};
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, LogitPaging, StateCheckpoint, StateTopology,
    synthetic_state_checkpoint_with,
};

fn synthetic_dense(topology: StateTopology, seed: u64) -> StateCheckpoint {
    synthetic_state_checkpoint_with(topology, seed)
}

#[test]
fn greedy_paged_decode_equals_single_page_decode_small_vocab() {
    let single = StateTopology {
        logit_paging: LogitPaging::SinglePage,
        ..StateTopology::ARM_B
    };
    let paged = StateTopology {
        logit_paging: LogitPaging::Paged,
        ..StateTopology::ARM_B
    };
    let ls = IntStateLoweredModel::lower(&synthetic_dense(single, 51)).expect("lowers single");
    let lp = IntStateLoweredModel::lower(&synthetic_dense(paged, 51)).expect("lowers paged");

    let mut ss = ls.zero_state();
    let mut sp = lp.zero_state();
    let mut input = 3u8;
    let mut cases_single = Vec::new();
    let mut cases_paged = Vec::new();
    for pos in 0..12usize {
        cases_single.push((pos, input, ss.clone()));
        cases_paged.push((pos, input, sp.clone()));
        let ts = ls.forward(input, &mut ss);
        let tp = lp.forward(input, &mut sp);
        assert_eq!(ts.argmax_full, tp.argmax_full, "argmax pos {pos}");
        input = tp.argmax;
    }

    let rs = run_state_rom_gate(&ls, &cases_single).expect("single gate");
    let rp = run_state_rom_gate(&lp, &cases_paged).expect("paged gate");
    assert!(rs.all_byte_exact, "single-page byte-exact: {:?}", rs.runs);
    assert!(rp.all_byte_exact, "paged byte-exact: {:?}", rp.runs);
    for (a, b) in rs.runs.iter().zip(rp.runs.iter()) {
        assert_eq!(a.rom_argmax, b.rom_argmax, "same argmax id single vs paged");
    }
}

#[test]
fn paged_decode_on_vocab_1024_is_byte_exact() {
    let t = StateTopology::D1024_DENSE;
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(t, 99)).expect("lowers");
    assert_eq!(lowered.topology.logit_paging, LogitPaging::Paged);

    let mut state = lowered.zero_state();
    let mut input = 7u8;
    let mut cases = Vec::new();
    for pos in 0..6usize {
        cases.push((pos, input, state.clone()));
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax; // low-byte feedback (see paged epilogue note)
        assert!(trace.argmax_full < t.vocab);
    }

    let report = run_state_rom_gate(&lowered, &cases).expect("paged 1024 gate");
    for run in &report.runs {
        assert!(
            run.byte_exact,
            "input {} pos {}: {:?}",
            run.input_id, run.state_from_position, run.mismatches
        );
    }
    assert!(
        report.all_byte_exact,
        "paged 1024 all byte-exact (covers argmax16, last logit page, heap)"
    );
}

#[test]
fn topk_heap_sampling_parity_host_vs_golden() {
    let t = StateTopology::D1024_DENSE;
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(t, 1234)).expect("lowers");
    let step = lowered.logit_dequant_step();
    let layout = StateWramLayout::plan(t, lowered.down_width, false).expect("plan");
    let pg = layout.paged.expect("paged layout");

    for k in [8u8, 40u8] {
        // SamplerConfig caps k at MAX_TOP_K=8; the ROM heap capacity is
        // min(HEAP_K_MAX, vocab)=40 regardless. The single-page config only
        // supplies scale_q16 + the (<=8) draw-candidate count; the paged draw
        // walks all `heap_count` finalized candidates. We use the same scale
        // for both k values and drive the golden over the full finalized heap.
        let cfg = SamplerConfig::from_temperature(8, step, 1.0).expect("temperature");
        let scale_q16 = cfg.scale_q16();
        let seed: u16 = 0xC0DE;

        let rom = build_state_multi_token_sampling_rom(&lowered, 1, &cfg)
            .expect("paged sampling rom builds");

        // Host golden.
        let mut state = lowered.zero_state();
        let trace = lowered.forward(0u8, &mut state);
        let cands: Vec<(i32, usize)> = trace.topk_heap.iter().map(|e| (e.logit, e.id)).collect();
        let mut rng = XorShift16::new(seed);
        let golden = sample_topk_from_candidates_trace(&cands, scale_q16, &mut rng);

        // Device.
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .trace_drop_policy(TraceDropPolicy::HaltAndError)
            .load_rom(&rom.rom)
            .expect("load rom");
        emu.poke(S_INPUT_ADDR, 0).expect("poke input");
        emu.poke(S_RNG_ADDR, (seed & 0xFF) as u8).expect("seed lo");
        emu.poke(S_RNG_ADDR + 1, (seed >> 8) as u8)
            .expect("seed hi");
        let budget = CycleBudget::Clock(ClockCycles(
            lowered
                .topology
                .macs_per_token()
                .saturating_mul(4096)
                .max(50_000_000),
        ));
        match emu.run_fast_until_pc(rom.token_end_pc, budget) {
            Ok(RunOutcome::TrapHit { .. }) => {}
            other => panic!("k={k}: did not reach token end: {other:?}"),
        }

        // Heap WRAM parity (heap_id u16 LE, ascending selection order).
        let count = golden.candidates.len();
        for (j, cand) in golden.candidates.iter().enumerate() {
            // ROM keeps the heap ascending (worst at 0); golden candidate j
            // (best-first) sits at ROM slot count-1-j.
            let slot = count - 1 - j;
            let id_addr = pg.heap_id + 2 * slot as u16;
            let lo = emu.peek(id_addr).expect("heap id lo");
            let hi = emu.peek(id_addr + 1).expect("heap id hi");
            let rom_id = u16::from(lo) | (u16::from(hi) << 8);
            assert_eq!(rom_id as usize, cand.id, "k={k} heap slot {slot} id");
        }

        let sampled = emu.peek(S_SAMPLED_ADDR).expect("peek sampled");
        assert_eq!(
            sampled,
            (golden.picked & 0xFF) as u8,
            "k={k}: device S_SAMPLED low byte == golden picked low byte"
        );
    }
}
