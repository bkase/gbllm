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
//!   golden for a fixed seed, k in {4, 8}.

use gbf_bench::one_token::DMG_M_CYCLES_PER_SECOND;
use gbf_bench::stateful::run_state_rom_gate;
use gbf_emu::{
    BootMode, ClockCycles, CycleBudget, DeterminismPolicy, Emulator, RunOutcome, TraceDropPolicy,
};
use gbf_kernel::asm_impl_shell::{SUBWORD_FONT_BYTES, build_state_subword_demo_rom};
use gbf_kernel::asm_impl_state::{
    PagedHeadStorage, S_ARGMAX_ADDR, S_INPUT_ADDR, S_INPUT_HI_ADDR, S_RNG_ADDR, S_SAMPLED_ADDR,
    SRAM_FULL_LOGITS_BASE, StateWramLayout, WeightLowering, build_state_multi_token_sampling_rom,
    build_state_multi_token_sampling_rom_with_paged_head_storage,
    build_state_one_token_rom_lowered, build_state_one_token_rom_with_paged_head_storage,
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
fn sram_full_head_preserves_every_i24_logit_argmax_heap_and_is_faster() {
    let t = StateTopology::D1024_DENSE;
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(t, 0x5a4d)).expect("lowers");
    let streamed = build_state_one_token_rom_lowered(&lowered, WeightLowering::V3)
        .expect("streamed reference ROM builds");
    let full = build_state_one_token_rom_with_paged_head_storage(
        &lowered,
        WeightLowering::V3,
        PagedHeadStorage::SramFull,
    )
    .expect("SRAM full-head ROM builds");
    assert_eq!(streamed.rom[0x0147], 0x19);
    assert_eq!((full.rom[0x0147], full.rom[0x0149]), (0x1A, 0x02));

    let input = 7u8;
    let mut host_state = lowered.zero_state();
    let trace = lowered.forward_at(usize::from(input), &mut host_state);
    let expected_full_logits: Vec<u8> = trace
        .logit_pages
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes()[..3].to_vec())
        .collect();
    assert_eq!(expected_full_logits.len(), 3 * t.vocab);

    let run = |rom: &gbf_kernel::asm_impl_state::StateOneTokenRom| {
        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .trace_drop_policy(TraceDropPolicy::HaltAndError)
            .load_rom(&rom.rom)
            .expect("load ROM");
        emu.poke(S_INPUT_ADDR, input).expect("poke input low");
        emu.poke(S_INPUT_HI_ADDR, 0).expect("poke input high");
        for slot in 0..t.state_slots {
            for k in 0..4u16 {
                emu.poke(rom.layout.state + (4 * slot) as u16 + k, 0)
                    .expect("zero state");
            }
        }
        let budget = CycleBudget::Clock(ClockCycles(1_000_000_000));
        assert!(matches!(
            emu.run_fast_until_pc(rom.token_start_pc, budget),
            Ok(RunOutcome::TrapHit { .. })
        ));
        if rom.rom[0x0147] == 0x1A {
            // Poison the entire full-logit range after the boot prologue has
            // enabled SRAM but before token execution. Parity therefore proves
            // the epilogue explicitly zeroes every i24 accumulator; it cannot
            // pass by relying on emulator/power-on RAM contents.
            for offset in 0..3 * t.vocab {
                emu.poke(SRAM_FULL_LOGITS_BASE + offset as u16, 0xA5 ^ offset as u8)
                    .expect("poison SRAM logit byte");
            }
        }
        let start = emu.m_cycle_count_floor().0;
        assert!(matches!(
            emu.run_fast_until_pc(rom.token_end_pc, budget),
            Ok(RunOutcome::TrapHit { .. })
        ));
        let cycles = emu.m_cycle_count_floor().0.saturating_sub(start);
        (emu, cycles)
    };

    let (_streamed_emu, streamed_cycles) = run(&streamed);
    let (full_emu, full_cycles) = run(&full);
    let sram_logits = full_emu
        .peek_range(SRAM_FULL_LOGITS_BASE, 3 * t.vocab)
        .expect("read full SRAM logits");
    assert_eq!(
        sram_logits, expected_full_logits,
        "every raw i24 logit is host-byte-identical"
    );

    let pg = full.layout.paged.expect("paged layout");
    let argmax_lo = full_emu.peek(S_ARGMAX_ADDR).expect("argmax low");
    let argmax_hi = full_emu
        .peek(gbf_kernel::asm_impl_state::S_SAMPLED_HI_ADDR)
        .expect("argmax high");
    assert_eq!(
        usize::from(u16::from_le_bytes([argmax_lo, argmax_hi])),
        trace.argmax_full,
        "u16 argmax parity"
    );
    let expected_last_page: Vec<u8> = trace
        .logits
        .iter()
        .flat_map(|v| v.to_le_bytes()[..3].to_vec())
        .collect();
    assert_eq!(
        full_emu
            .peek_range(full.layout.logits, expected_last_page.len())
            .expect("last WRAM logit page"),
        expected_last_page,
        "final page remains resident for the existing gate"
    );
    assert_eq!(
        usize::from(full_emu.peek(pg.heap_count).expect("heap count")),
        trace.topk_heap.len()
    );
    for (host_rank, host) in trace.topk_heap.iter().enumerate() {
        let slot = trace.topk_heap.len() - 1 - host_rank;
        let logit_bytes = full_emu
            .peek_range(pg.heap_logit + 3 * slot as u16, 3)
            .expect("heap logit");
        let sign = if logit_bytes[2] & 0x80 == 0 { 0 } else { 0xFF };
        let logit = i32::from_le_bytes([logit_bytes[0], logit_bytes[1], logit_bytes[2], sign]);
        let id_bytes = full_emu
            .peek_range(pg.heap_id + 2 * slot as u16, 2)
            .expect("heap id");
        assert_eq!(
            (
                logit,
                usize::from(u16::from_le_bytes([id_bytes[0], id_bytes[1]]))
            ),
            (host.logit, host.id)
        );
    }
    assert!(
        full_cycles < streamed_cycles,
        "LUT hoist must reduce cycles: full={full_cycles}, streamed={streamed_cycles}"
    );
    eprintln!(
        "paged head cycles (M): streamed={streamed_cycles}, SRAM-full={full_cycles}, speedup={:.2}x",
        streamed_cycles as f64 / full_cycles as f64
    );
}

#[test]
fn sram_full_head_sampling_k4_k8_matches_multitoken_host_sequence_and_state() {
    let t = StateTopology::D1024_DENSE;
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(t, 0x51a9)).expect("lowers");
    let step = lowered.logit_dequant_step();
    let seed = 0xBEEFu16;
    let n_tokens = 3u16;

    for k in [4u8, 8u8] {
        let cfg = SamplerConfig::from_temperature(k, step, 0.8).expect("sampler");
        let rom = build_state_multi_token_sampling_rom_with_paged_head_storage(
            &lowered,
            n_tokens,
            &cfg,
            WeightLowering::V3,
            PagedHeadStorage::SramFull,
        )
        .expect("SRAM sampling ROM builds");

        let mut host_state = lowered.zero_state();
        let mut rng = XorShift16::new(seed);
        let mut input = 0usize;
        let mut host_ids = Vec::new();
        for _ in 0..n_tokens {
            let trace = lowered.forward_at(input, &mut host_state);
            let cands: Vec<(i32, usize)> = trace
                .topk_heap
                .iter()
                .take(usize::from(k))
                .map(|e| (e.logit, e.id))
                .collect();
            let pick = sample_topk_from_candidates_trace(&cands, cfg.scale_q16(), &mut rng).picked;
            host_ids.push(pick as u16);
            input = pick;
        }

        let mut emu = Emulator::builder()
            .boot_mode(BootMode::PostBootDmg)
            .policy(DeterminismPolicy::default())
            .trace_drop_policy(TraceDropPolicy::HaltAndError)
            .load_rom(&rom.rom)
            .expect("load SRAM sampling ROM");
        emu.poke(S_INPUT_ADDR, 0).expect("input low");
        emu.poke(S_INPUT_HI_ADDR, 0).expect("input high");
        emu.poke(S_RNG_ADDR, seed as u8).expect("seed low");
        emu.poke(S_RNG_ADDR + 1, (seed >> 8) as u8)
            .expect("seed high");
        let budget = CycleBudget::Clock(ClockCycles(1_000_000_000));
        assert!(matches!(
            emu.run_fast_until_pc(rom.token_start_pc, budget),
            Ok(RunOutcome::TrapHit { .. })
        ));
        // Poison before token 0; each token must independently clear the full
        // vector before accumulating its lane contributions.
        for offset in 0..3 * t.vocab {
            emu.poke(SRAM_FULL_LOGITS_BASE + offset as u16, 0x3C)
                .expect("poison SRAM");
        }
        assert!(matches!(
            emu.run_fast_until_pc(rom.token_end_pc, budget),
            Ok(RunOutcome::TrapHit { .. })
        ));

        let device_ids = emu
            .peek_range(rom.layout.out, usize::from(n_tokens))
            .expect("output ring");
        assert_eq!(
            device_ids,
            host_ids.iter().map(|id| *id as u8).collect::<Vec<_>>(),
            "k={k} sampled low-byte sequence"
        );
        let expected_state: Vec<u8> = host_state.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(
            emu.peek_range(rom.layout.state, expected_state.len())
                .expect("final recurrent state"),
            expected_state,
            "k={k} final state proves full-u16 sampled feedback parity"
        );
        assert_eq!(
            emu.peek(S_SAMPLED_ADDR).expect("sampled low"),
            *host_ids.last().expect("three host ids") as u8
        );
        assert_eq!(
            emu.peek(gbf_kernel::asm_impl_state::S_SAMPLED_HI_ADDR)
                .expect("sampled high"),
            (*host_ids.last().expect("three host ids") >> 8) as u8
        );
    }
}

#[test]
fn dense_d192_v1024_full_head_is_host_exact_and_under_30_seconds_per_token() {
    let topology = StateTopology {
        vocab: 1024,
        logit_paging: LogitPaging::Paged,
        ..StateTopology::D192
    };
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(topology, 0x1925))
        .expect("dense d192/V1024 lowers");
    let cfg = SamplerConfig::new(8, 2253).expect("sampler");
    let font = vec![0u8; SUBWORD_FONT_BYTES];
    let id_bytes = vec![vec![b'x']; topology.vocab];
    let rom = build_state_subword_demo_rom(&lowered, &cfg, 1, &font, &id_bytes)
        .expect("production dense subword ROM builds");
    assert_eq!(rom.paged_head_storage, PagedHeadStorage::SramFull);
    assert_eq!(rom.weight_lowering, WeightLowering::V3);

    let prompt_id = 7u16;
    let mut host_state = lowered.zero_state();
    let host = lowered.forward_at(usize::from(prompt_id), &mut host_state);
    let expected_logits: Vec<u8> = host
        .logit_pages
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes()[..3].to_vec())
        .collect();

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .expect("load production dense subword ROM");
    let budget = CycleBudget::Clock(ClockCycles(DMG_M_CYCLES_PER_SECOND * 4 * 60));
    assert!(matches!(
        emu.run_fast_until_pc(rom.idle_pc, budget),
        Ok(RunOutcome::TrapHit { .. })
    ));
    emu.poke(rom.prompt_ids_addr, prompt_id as u8)
        .expect("prompt low");
    emu.poke(rom.prompt_ids_addr + 1, (prompt_id >> 8) as u8)
        .expect("prompt high");
    emu.poke(rom.prompt_len_addr, 1).expect("prompt length");
    emu.poke(rom.go_addr, 1).expect("start demo");
    emu.poke(S_RNG_ADDR, 0xED).expect("seed low");
    emu.poke(S_RNG_ADDR + 1, 0x5E).expect("seed high");
    assert!(matches!(
        emu.run_fast_until_pc(rom.forward_pass_pc, budget),
        Ok(RunOutcome::TrapHit { .. })
    ));
    // As in the compact parity gate, poison after cartridge init but before
    // the exact production forward pass.
    for offset in 0..3 * topology.vocab {
        emu.poke(SRAM_FULL_LOGITS_BASE + offset as u16, 0xD3)
            .expect("poison full logits");
    }
    let start = emu.m_cycle_count_floor().0;
    assert!(matches!(
        emu.run_fast_until_pc(rom.warm_boundary_pc, budget),
        Ok(RunOutcome::TrapHit { .. })
    ));
    let m_cycles = emu.m_cycle_count_floor().0.saturating_sub(start);
    let seconds = m_cycles as f64 / DMG_M_CYCLES_PER_SECOND as f64;

    assert_eq!(
        emu.peek_range(SRAM_FULL_LOGITS_BASE, expected_logits.len())
            .expect("read production full logits"),
        expected_logits,
        "all d192/V1024 logits are host-byte-identical"
    );
    let pg = rom.layout.paged.expect("paged layout");
    let argmax_bytes = emu.peek_range(pg.argmax16, 2).expect("argmax16");
    assert_eq!(
        usize::from(u16::from_le_bytes([argmax_bytes[0], argmax_bytes[1]])),
        host.argmax_full
    );
    assert!(
        seconds <= 30.0,
        "production dense token must fit 30 s: {m_cycles} M-cycles = {seconds:.3} s"
    );
    eprintln!(
        "dense d192/V1024 production: banks={}, driver={} B, {m_cycles} M-cycles = {seconds:.3} s/token",
        rom.bank_count, rom.driver_bytes
    );
}

#[test]
fn topk_heap_sampling_parity_host_vs_golden() {
    let t = StateTopology::D1024_DENSE;
    let lowered = IntStateLoweredModel::lower(&synthetic_dense(t, 1234)).expect("lowers");
    let step = lowered.logit_dequant_step();
    let layout = StateWramLayout::plan(t, lowered.down_width, false).expect("plan");
    let pg = layout.paged.expect("paged layout");

    for k in [4u8, 8u8] {
        // Paged sampling must retain and draw exactly the configured k, just
        // like the single-page sampler. The host trace keeps an audit top-40;
        // truncate that finalized selection order to the configured k.
        let cfg = SamplerConfig::from_temperature(k, step, 1.0).expect("temperature");
        let scale_q16 = cfg.scale_q16();
        let seed: u16 = 0xC0DE;

        let rom = build_state_multi_token_sampling_rom(&lowered, 1, &cfg)
            .expect("paged sampling rom builds");

        // Host golden.
        let mut state = lowered.zero_state();
        let trace = lowered.forward(0u8, &mut state);
        let cands: Vec<(i32, usize)> = trace
            .topk_heap
            .iter()
            .take(usize::from(cfg.k()))
            .map(|e| (e.logit, e.id))
            .collect();
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
