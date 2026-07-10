//! Wide-vocabulary subword demo surface gate: dense and MoE students generate
//! multi-char text on-device, host-byte-identically.
//!
//! Byte-identity is the contract, mirroring `demo`'s charset acceptance:
//! (a) the on-device generated token-id sequence == the host mirror
//!     (`subword_host_generate`), and
//! (b) the rendered transcript BG bytes == the host id_bytes -> tile map of the
//!     decoded token stream (`expected_subword_transcript_bg`), i.e. each token
//!     paints its MULTIPLE literal bytes (one token -> several chars).
//!
//! The always-on test uses a SMALL synthetic Paged MoE checkpoint with a vocab
//! that spans past 255 (so the wide u16 embedding-feedback + render path is
//! exercised); the router is correct-but-slow, so the token count is kept small
//! (>= 8). Real bridged students are behind `MOE_PARITY_DIR` or
//! `DENSE_PARITY_DIR`.

use std::path::PathBuf;

use gbf_bench::one_token::DMG_M_CYCLES_PER_SECOND;
use gbf_bench::stateful::load_state_checkpoint;
use gbf_bench::subword_demo::{
    run_subword_demo_session, subword_font_tiles, subword_host_generate, synthetic_id_bytes,
};
use gbf_kernel::asm_impl_shell::build_state_subword_demo_rom;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, LogitPaging, StateTopology, synthetic_moe_state_checkpoint,
};

/// A small Paged MoE topology whose vocab spans past 255, so the demo exercises
/// (1) wide u16 embedding feedback, (2) the paged head/heap, (3) MoE routing,
/// and (4) multi-char id_bytes render — while staying small enough that a
/// >= 8-token generation finishes in a reasonable time.
fn synthetic_paged_moe() -> IntStateLoweredModel {
    let topo = StateTopology {
        d_model: 32,
        d_ff: 64,
        n_blocks: 2,
        state_slots: 32,
        vocab: 300,
        n_experts: 2,
        logit_paging: LogitPaging::Paged,
    };
    topo.validate()
        .expect("synthetic paged MoE topology is valid");
    let ck = synthetic_moe_state_checkpoint(topo, 0x5ab0);
    IntStateLoweredModel::lower(&ck).expect("synthetic paged MoE checkpoint lowers")
}

#[test]
fn synthetic_subword_moe_demo_renders_byte_identically_to_host() {
    let lowered = synthetic_paged_moe();
    assert!(lowered.topology.is_moe(), "paged MoE topology");
    assert_eq!(lowered.topology.logit_paging, LogitPaging::Paged);
    assert!(
        lowered.topology.vocab > 256,
        "vocab spans past the u8 id space"
    );

    let id_bytes = synthetic_id_bytes(lowered.topology.vocab);
    let font = subword_font_tiles();
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(8, step, 0.8).expect("valid sampler");

    let n_gen = 12u8; // small: the router is correct-but-slow
    let rom = build_state_subword_demo_rom(&lowered, &cfg, n_gen, &font, &id_bytes)
        .expect("subword MoE demo ROM builds");
    assert!(rom.bank_count > 1);
    assert!(rom.id_bytes_geom.stride.is_power_of_two());

    // Host-encoded prompt: ids that span past 255 (wide feedback in warmup too).
    let prompt_ids: Vec<u16> = vec![3, 260, 41, 288];
    let rng_seed = 0x5EEDu16;

    // The two gates: byte-identical sequence AND byte-identical rendered
    // transcript. `run_subword_demo_session` checks the transcript against the
    // host id_bytes render of the host mirror's generated sequence.
    let result = run_subword_demo_session(&rom, &lowered, &cfg, &id_bytes, &prompt_ids, rng_seed)
        .expect("scripted subword demo session runs");

    assert!(result.n_tokens >= 8, "sustained generation (>= 8 tokens)");
    assert!(
        result.transcript_bg_ok,
        "on-device transcript render != host id_bytes render; first mismatch {:?}",
        result.bg_first_mismatch,
    );
    assert!(
        result.sequence_matches,
        "on-device ids {:?} != host ids {:?}",
        result.device_sequence, result.host_sequence
    );

    // The rendered transcript is a MULTI-CHAR expansion of the token ids: at
    // least one generated token expands to more than one byte (ids >= 256).
    let host_seq = &result.host_sequence;
    let multichar = host_seq
        .iter()
        .any(|&id| id_bytes[usize::from(id)].len() > 1);
    let total_bytes: usize = host_seq
        .iter()
        .map(|&id| id_bytes[usize::from(id)].len())
        .sum();
    assert!(
        multichar || total_bytes > host_seq.len(),
        "at least one token renders multiple chars (subword expansion)",
    );

    // Determinism: a second scripted session is byte-identical.
    let rerun = run_subword_demo_session(&rom, &lowered, &cfg, &id_bytes, &prompt_ids, rng_seed)
        .expect("second session runs");
    assert_eq!(
        result.transcript_sha256, rerun.transcript_sha256,
        "transcript byte-identical across runs",
    );
    assert_eq!(result.host_sequence, rerun.host_sequence);
    assert_eq!(result.device_sequence, rerun.device_sequence);

    println!(
        "synthetic subword demo: {} tokens, {} transcript bytes, sample: {:?}",
        result.n_tokens,
        total_bytes,
        result.decoded_text.chars().take(60).collect::<String>(),
    );
}

fn moe_parity_dir() -> Option<PathBuf> {
    std::env::var("MOE_PARITY_DIR")
        .or_else(|_| std::env::var("MOE_INT_DIR"))
        .ok()
        .map(PathBuf::from)
}

fn dense_parity_dir() -> Option<PathBuf> {
    std::env::var("DENSE_PARITY_DIR").ok().map(PathBuf::from)
}

fn run_real_subword_student_demo(root: &PathBuf, expected_moe: bool, top_k: u8, temperature: f64) {
    let ckpt = root.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)
        .unwrap_or_else(|e| panic!("load real student at {}: {e}", ckpt.display()));
    let topo = bundle.topology;
    assert_eq!(
        topo.is_moe(),
        expected_moe,
        "unexpected topology n_experts={}",
        topo.n_experts
    );
    assert_eq!(topo.logit_paging, LogitPaging::Paged, "vocab-1024 is Paged");
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .unwrap_or_else(|e| panic!("lower real student: {e}"));

    let bpe_path = root.join("tokenizer/gbllm_bpe.v2.json");
    let bpe_text = std::fs::read_to_string(&bpe_path)
        .unwrap_or_else(|e| panic!("read BPE artifact {}: {e}", bpe_path.display()));
    let bpe = gbf_data::bpe::BpeModel::from_json(&bpe_text)
        .unwrap_or_else(|e| panic!("parse BPE artifact: {e}"));
    assert_eq!(bpe.vocab_size(), topo.vocab, "BPE vocab matches the model");
    let id_bytes: Vec<Vec<u8>> = (0..topo.vocab)
        .map(|id| bpe.id_bytes(id as u16).expect("id in vocab").to_vec())
        .collect();

    let cfg = SamplerConfig::from_temperature(top_k, lowered.logit_dequant_step(), temperature)
        .expect("valid sampler");
    let prompt = "Once upon a time";
    let prompt_ids = bpe.encode(prompt);
    assert!(!prompt_ids.is_empty());

    let n_gen = 12u8;
    let rom = build_state_subword_demo_rom(&lowered, &cfg, n_gen, &subword_font_tiles(), &id_bytes)
        .expect("real subword demo ROM builds within budget");
    println!(
        "real subword demo ROM: banks={} driver={} storage={:?} ui={} id_bytes_stride={} table={}",
        rom.bank_count,
        rom.driver_bytes,
        rom.paged_head_storage,
        rom.ui_bank_bytes,
        rom.id_bytes_geom.stride,
        rom.table_bytes,
    );

    let rng_seed = 0x5EEDu16;
    let host_seq = subword_host_generate(&lowered, &cfg, &id_bytes, &prompt_ids, rng_seed, n_gen);
    let result = run_subword_demo_session(&rom, &lowered, &cfg, &id_bytes, &prompt_ids, rng_seed)
        .expect("real scripted subword demo session runs");
    assert_eq!(
        result.host_sequence, host_seq,
        "host generation is deterministic"
    );
    assert_eq!(
        result.device_sequence, host_seq,
        "real device sampled ids must match the host mirror"
    );
    assert!(
        result.transcript_bg_ok,
        "REAL on-device transcript != host BPE render; first mismatch {:?}",
        result.bg_first_mismatch,
    );
    let host_text = bpe.decode(&result.host_sequence);
    println!("real subword generation: prompt={prompt:?}");
    println!("  ids    = {:?}", result.host_sequence);
    println!("  decode = {host_text:?}");
    println!("  cycles = {:?}", result.generation_m_cycles);
    assert_eq!(
        result.decoded_text, host_text,
        "rendered transcript == BPE decode"
    );
    if !expected_moe {
        let worst = result
            .generation_m_cycles
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        assert!(
            worst <= 30 * DMG_M_CYCLES_PER_SECOND,
            "real dense demo exceeds 30 s/token: {worst} M-cycles = {:.3} s",
            worst as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        );
    }
}

/// THE REAL MILESTONE (env-gated like the other MoE gates): build the REAL
/// bridged subword MoE student's demo ROM (vocab-1024 Paged, 8 experts) and
/// assert it GENERATES + RENDERS byte-identically to the host — the on-device
/// transcript equals `BpeModel::decode(host_generated_ids)` mapped to tiles.
///
/// Set `MOE_PARITY_DIR` (or `MOE_INT_DIR`) to a dir with `ckpt/`
/// (`f_s8_moe_state_checkpoint_export.v2`) and `tokenizer/gbllm_bpe.v2.json`
/// (the deployed BPE artifact).
#[test]
#[ignore = "requires MOE_PARITY_DIR pointing at a real bridged MoE student + BPE artifact"]
fn real_subword_moe_student_demo_renders_byte_identically_to_host() {
    let Some(root) = moe_parity_dir() else {
        eprintln!("MOE_PARITY_DIR / MOE_INT_DIR unset; skipping");
        return;
    };
    run_real_subword_student_demo(&root, true, 8, 0.8);
}

#[test]
#[ignore = "requires DENSE_PARITY_DIR pointing at a real bridged dense student + BPE artifact"]
fn real_dense_subword_student_meets_quality_parity_and_latency_budget() {
    let Some(root) = dense_parity_dir() else {
        eprintln!("DENSE_PARITY_DIR unset; skipping");
        return;
    };
    run_real_subword_student_demo(&root, false, 4, 0.6);
}

#[test]
fn probe_paged_moe_multitoken_gate() {
    // Confirms the SAME synthetic paged MoE topology generates on-device via the
    // existing byte-exact MoE ROM gate (reaches token boundaries), isolating any
    // demo-ROM-specific hang from a model-forward problem.
    let lowered = synthetic_paged_moe();
    let seed = 3u8;
    let cases = vec![(0usize, seed, lowered.zero_state())];
    let report = gbf_bench::stateful::run_state_moe_rom_gate_lowered(&lowered, &cases, seed, 4)
        .expect("paged MoE multi-token gate runs");
    println!(
        "probe: sequences_match={} banks={}",
        report.generation.sequences_match, report.bank_count
    );
    assert!(report.all_byte_exact);
}
