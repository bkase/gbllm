//! End-to-end acceptance for the playable subword cartridge: the prompt must
//! enter through real JOYP frames, tokenize on device exactly like the deployed
//! BPE artifact, and drive the same full-u16 generation as the host mirror.

use std::path::PathBuf;

use gbf_bench::one_token::DMG_M_CYCLES_PER_SECOND;
use gbf_bench::stateful::{id_to_char, load_state_checkpoint};
use gbf_bench::subword_demo::{
    expected_subword_transcript_bg, run_typed_subword_shell_session, subword_font_tiles,
    subword_typing_script,
};
use gbf_data::bpe::{BpeModel, pretokenize};
use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, JoypadFrame,
    Predicate, RunOutcome, TrapAction, TrapKind,
};
use gbf_hw::joypad::Button;
use gbf_kernel::asm_impl_shell::{
    BG_MAP_BASE, BG_MAP_STRIDE, PROMPT_ROW, SHELL_PROMPT_CAP, SUBWORD_CURSOR_TILE,
    SUBWORD_KEY_BYTES, SUBWORD_SPACE_BYTE, TRANSCRIPT_COLS, build_state_subword_shell_rom,
};
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, LogitPaging, StateTopology, synthetic_state_checkpoint_with,
};

fn keyboard_bytes() -> Vec<u8> {
    (0..76u8).map(|id| id_to_char(id) as u8).collect()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-bench has workspace parent")
        .to_path_buf()
}

fn deployed_bpe() -> BpeModel {
    let path = workspace_root().join("training/artifacts/tinystories_bpe_1024.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read deployed BPE {}: {error}", path.display()));
    BpeModel::from_json(&text).expect("deployed BPE validates")
}

fn id_bytes(bpe: &BpeModel) -> Vec<Vec<u8>> {
    (0..bpe.vocab_size())
        .map(|id| {
            bpe.id_bytes(id as u16)
                .expect("id in deployed vocabulary")
                .to_vec()
        })
        .collect()
}

fn synthetic_typed_model(vocab: usize) -> IntStateLoweredModel {
    let topology = StateTopology {
        d_model: 16,
        d_ff: 32,
        n_blocks: 1,
        state_slots: 16,
        vocab,
        n_experts: 1,
        logit_paging: LogitPaging::Paged,
    };
    topology.validate().expect("small wide topology is valid");
    let checkpoint = synthetic_state_checkpoint_with(topology, 0x0051_ab1e);
    IntStateLoweredModel::lower(&checkpoint).expect("synthetic typed model lowers")
}

fn run_past_to(emu: &mut Emulator, pc: u16, budget: CycleBudget, phase: &str) {
    emu.step()
        .unwrap_or_else(|error| panic!("step past {phase}: {error}"));
    let outcome = emu
        .run_fast_until_pc(pc, budget)
        .unwrap_or_else(|error| panic!("run to {phase}: {error}"));
    assert!(
        matches!(outcome, RunOutcome::TrapHit { kind: TrapKind::Pc { addr }, .. } if addr == pc),
        "did not reach {phase} at {pc:#06x}: {outcome:?}"
    );
}

fn tap_at_idle(emu: &mut Emulator, idle_pc: u16, button: Button, budget: CycleBudget) {
    emu.set_joypad(JoypadFrame::pressed(button));
    run_past_to(emu, idle_pc, budget, "idle after button press");
    emu.set_joypad(JoypadFrame::default());
    run_past_to(emu, idle_pc, budget, "idle after button release");
}

#[test]
fn typed_prompt_script_uses_press_release_edges_and_rejects_absent_bytes() {
    let keys = keyboard_bytes();
    assert_eq!(
        keys, SUBWORD_KEY_BYTES,
        "host typing planner and cartridge keyboard cannot drift"
    );
    let prompt = b"Once 8\n TA\nz";
    let script = subword_typing_script(prompt, &keys).expect("every prompt byte has a key");

    assert!(
        script.len() > prompt.len() * 2,
        "script includes D-pad navigation as well as one A edge per byte"
    );
    assert_eq!(script.len() % 2, 0, "every press has a release frame");
    for pair in script.chunks_exact(2) {
        assert_ne!(pair[0].bits(), 0, "first frame is a real button press");
        assert_eq!(pair[1].bits(), 0, "second frame releases every button");
    }
    assert_eq!(
        script
            .iter()
            .filter(|frame| frame.is_pressed(Button::A))
            .count(),
        prompt.len(),
        "A enters every prompt byte exactly once"
    );
    assert!(
        subword_typing_script(b"unsupported\0byte", &keys).is_none(),
        "the host cannot pretend to type bytes absent from the real keyboard"
    );
}

#[test]
fn transcript_display_replaces_unsupported_bytes_without_changing_layout() {
    let id_bytes = vec![vec![b'A', 0x01, 0x80, b'\n', 0x7f, b'Z']];
    let bg = expected_subword_transcript_bg(&[0], &id_bytes);
    let cols = usize::from(TRANSCRIPT_COLS);

    assert_eq!(&bg[..3], b"A??");
    assert_eq!(bg[cols], b'?', "DEL is not a printable glyph");
    assert_eq!(bg[cols + 1], b'Z');
    assert_eq!(bg[cols + 2], SUBWORD_CURSOR_TILE);
    assert!(
        bg[3..cols].iter().all(|&tile| tile == SUBWORD_SPACE_BYTE),
        "newline advances to the next transcript row"
    );
}

#[test]
fn interactive_controls_backspace_cap_empty_submit_and_reuse_real_joyp() {
    let bpe = deployed_bpe();
    let lowered = synthetic_typed_model(bpe.vocab_size());
    let bytes = id_bytes(&bpe);
    let sampler = SamplerConfig::from_temperature(4, lowered.logit_dequant_step(), 0.6)
        .expect("coherence-first sampler is valid");
    let rom = build_state_subword_shell_rom(
        &lowered,
        &sampler,
        1,
        &subword_font_tiles(),
        &bytes,
        bpe.merges(),
    )
    .expect("one-token interactive controls ROM builds");
    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .load_rom(&rom.rom)
        .expect("interactive controls ROM loads");
    let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    assert!(
        matches!(
            emu.run_fast_until_pc(rom.idle_pc, budget)
                .expect("boot to keyboard idle"),
            RunOutcome::TrapHit { kind: TrapKind::Pc { addr }, .. } if addr == rom.idle_pc
        ),
        "boot must reach keyboard idle"
    );

    // An empty START must return directly to idle. The tokenizer breakpoint
    // makes this fail if the empty-prompt guard is accidentally bypassed.
    let tokenize_trap = emu.traps().add_pc(
        rom.tokenize_done_pc,
        Predicate::Always,
        TrapAction::HaltAndReport,
    );
    tap_at_idle(&mut emu, rom.idle_pc, Button::Start, budget);
    assert!(emu.traps().remove(tokenize_trap));
    assert_eq!(
        emu.peek(rom.prompt_byte_len_addr)
            .expect("read empty prompt length"),
        0
    );
    assert_eq!(
        emu.peek(rom.prompt_token_len_addr)
            .expect("read empty token length"),
        0
    );

    // The cursor starts on `A`: enter AAA, delete one through B, then make
    // nineteen more attempts. Only eighteen may fit after the two survivors.
    for _ in 0..3 {
        tap_at_idle(&mut emu, rom.idle_pc, Button::A, budget);
    }
    tap_at_idle(&mut emu, rom.idle_pc, Button::B, budget);
    assert_eq!(
        emu.peek_range(rom.prompt_bytes_addr, 2)
            .expect("read prompt after backspace"),
        b"AA"
    );
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE;
    let row_after_backspace = emu
        .peek_range(prompt_bg, usize::from(TRANSCRIPT_COLS))
        .expect("read prompt row after backspace");
    assert_eq!(&row_after_backspace[..2], b"AA");
    assert!(
        row_after_backspace[2..]
            .iter()
            .all(|&tile| tile == SUBWORD_SPACE_BYTE),
        "B must erase the deleted prompt glyph"
    );

    for _ in 0..19 {
        tap_at_idle(&mut emu, rom.idle_pc, Button::A, budget);
    }
    assert_eq!(
        emu.peek(rom.prompt_byte_len_addr)
            .expect("read capped prompt length"),
        SHELL_PROMPT_CAP,
        "the twenty-first byte attempt must be ignored"
    );
    assert_eq!(
        emu.peek_range(rom.prompt_bytes_addr, usize::from(SHELL_PROMPT_CAP))
            .expect("read capped prompt"),
        vec![b'A'; usize::from(SHELL_PROMPT_CAP)]
    );
    assert_eq!(
        emu.peek_range(prompt_bg, usize::from(TRANSCRIPT_COLS))
            .expect("read full prompt row"),
        vec![b'A'; usize::from(SHELL_PROMPT_CAP)]
    );

    // Submit and cross a real generated-token boundary, then prove the same
    // cartridge accepts a fresh prompt with no stale bytes or echo tiles.
    emu.set_joypad(JoypadFrame::pressed(Button::Start));
    run_past_to(
        &mut emu,
        rom.tokenize_done_pc,
        budget,
        "tokenizer after full prompt",
    );
    emu.set_joypad(JoypadFrame::default());
    let token_len = emu
        .peek(rom.prompt_token_len_addr)
        .expect("read encoded token length");
    assert!(token_len > 0, "nonempty capped prompt must encode");
    for _ in 0..token_len {
        run_past_to(
            &mut emu,
            rom.warm_boundary_pc,
            budget,
            "prompt warm boundary",
        );
    }
    run_past_to(
        &mut emu,
        rom.token_boundary_pc,
        budget,
        "generated token boundary",
    );
    run_past_to(
        &mut emu,
        rom.gen_done_pc,
        budget,
        "one-token generation done",
    );
    run_past_to(
        &mut emu,
        rom.idle_pc,
        budget,
        "post-generation keyboard idle",
    );
    assert_eq!(
        emu.peek(rom.prompt_byte_len_addr)
            .expect("read reset prompt length"),
        0
    );
    assert_eq!(
        emu.peek(rom.prompt_token_len_addr)
            .expect("read reset token length"),
        0
    );
    assert!(
        emu.peek_range(prompt_bg, usize::from(TRANSCRIPT_COLS))
            .expect("read reset prompt row")
            .iter()
            .all(|&tile| tile == SUBWORD_SPACE_BYTE),
        "generation completion must clear the entire input row"
    );

    tap_at_idle(&mut emu, rom.idle_pc, Button::A, budget);
    assert_eq!(
        emu.peek(rom.prompt_byte_len_addr)
            .expect("read reused prompt length"),
        1
    );
    assert_eq!(
        emu.peek(rom.prompt_bytes_addr)
            .expect("read reused prompt byte"),
        b'A'
    );
    let reused_row = emu
        .peek_range(prompt_bg, usize::from(TRANSCRIPT_COLS))
        .expect("read reused prompt row");
    assert_eq!(reused_row[0], b'A');
    assert!(
        reused_row[1..]
            .iter()
            .all(|&tile| tile == SUBWORD_SPACE_BYTE),
        "fresh input row must contain only the new byte"
    );
}

#[test]
fn joypad_typed_prompt_bpe_and_u16_generation_match_host_exactly() {
    let bpe = deployed_bpe();
    let lowered = synthetic_typed_model(bpe.vocab_size());
    let bytes = id_bytes(&bpe);
    let sampler = SamplerConfig::from_temperature(4, lowered.logit_dequant_step(), 0.6)
        .expect("coherence-first sampler is valid");
    let rom = build_state_subword_shell_rom(
        &lowered,
        &sampler,
        3,
        &subword_font_tiles(),
        &bytes,
        bpe.merges(),
    )
    .expect("interactive subword ROM builds");
    assert_eq!(
        rom.rng_seed, 0x5EED,
        "production default seed is baked into ROM"
    );

    // This combines a real merged token (`Once` -> 435) with punctuation,
    // digit, whitespace/newline, and letter boundaries. A broken global merge emits
    // token 270 across the `\n ` / `TA` chunk boundary; canonical BPE must not.
    const PROMPT: &str = "Once!? 8\n TA\nz";
    assert_eq!(
        pretokenize(PROMPT),
        vec!["Once", "!?", " 8", "\n ", "TA", "\n", "z"],
        "the test deliberately crosses every relevant ASCII chunk boundary"
    );
    assert_eq!(
        bpe.encode(PROMPT),
        vec![435, 33, 63, 32, 56, 10, 32, 84, 65, 10, 122],
        "literal artifact oracle catches accidental global merging"
    );

    let result = run_typed_subword_shell_session(&rom, &lowered, &sampler, &bpe, &bytes, PROMPT)
        .expect("button-driven typed-subword session completes");
    assert!(
        result.host_poke_addresses.is_empty(),
        "production interaction must be JOYP-only, saw host pokes {:?}",
        result.host_poke_addresses
    );
    assert!(
        result.rng_seed_initialized,
        "ROM initializes its own RNG seed"
    );
    assert!(result.prompt_bytes_match, "JOYP-entered WRAM bytes drifted");
    assert!(result.prompt_echo_ok, "visible ASCII prompt echo drifted");
    assert_eq!(result.device_prompt_ids, result.host_prompt_ids);
    assert!(
        result.device_prompt_ids.iter().any(|&id| id > 255),
        "on-device tokenizer preserves wide u16 IDs"
    );
    assert_eq!(
        result.generation.device_sequence, result.generation.host_sequence,
        "every sampled u16 ID must match the host mirror"
    );
    assert!(
        result.generation.transcript_bg_ok,
        "visible transcript drifted"
    );
    assert!(
        result.returned_to_idle,
        "shell did not return to keyboard idle"
    );
    assert!(
        result.prompt_reset_ok,
        "next prompt did not start from a clean row"
    );
    assert!(result.all_gates_pass());

    let worst = result
        .generation
        .generation_m_cycles
        .iter()
        .copied()
        .max()
        .expect("three generated token boundaries");
    assert!(
        worst <= 30 * DMG_M_CYCLES_PER_SECOND,
        "synthetic typed demo exceeds 30 s/token: {:.3} s",
        worst as f64 / DMG_M_CYCLES_PER_SECOND as f64
    );
}

#[test]
#[ignore = "requires DENSE_PARITY_DIR with the real dense d192 checkpoint and tokenizer"]
fn real_dense_joypad_typed_prompt_meets_parity_and_latency_contract() {
    let Some(root) = std::env::var("DENSE_PARITY_DIR").ok().map(PathBuf::from) else {
        eprintln!("DENSE_PARITY_DIR unset; skipping");
        return;
    };
    let bundle = load_state_checkpoint(&root.join("ckpt")).expect("load real dense checkpoint");
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint).expect("lower real checkpoint");
    let tokenizer_path = root.join("tokenizer/gbllm_bpe.v2.json");
    let bpe = BpeModel::from_json(
        &std::fs::read_to_string(&tokenizer_path).expect("read real tokenizer artifact"),
    )
    .expect("parse real tokenizer");
    assert_eq!(bpe.vocab_size(), lowered.topology.vocab);
    let bytes = id_bytes(&bpe);
    let sampler = SamplerConfig::from_temperature(4, lowered.logit_dequant_step(), 0.6)
        .expect("production sampler");
    let rom = build_state_subword_shell_rom(
        &lowered,
        &sampler,
        4,
        &subword_font_tiles(),
        &bytes,
        bpe.merges(),
    )
    .expect("real interactive cartridge builds");
    assert_eq!(rom.rng_seed, 0x5EED, "emitter and acceptance seed agree");
    let result =
        run_typed_subword_shell_session(&rom, &lowered, &sampler, &bpe, &bytes, "Once upon a time")
            .expect("real button-driven session completes");
    assert!(result.all_gates_pass(), "real typed session: {result:#?}");
    assert_eq!(result.host_prompt_ids, vec![435, 443, 258, 402]);

    let worst = result
        .generation
        .generation_m_cycles
        .iter()
        .copied()
        .max()
        .expect("real generation has token boundaries");
    assert!(
        worst <= 30 * DMG_M_CYCLES_PER_SECOND,
        "real interactive dense ROM exceeds 30 s/token: {:.3} s",
        worst as f64 / DMG_M_CYCLES_PER_SECOND as f64
    );
}
