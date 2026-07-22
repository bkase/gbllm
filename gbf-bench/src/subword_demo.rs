//! Wide-vocabulary subword demo host mirror and scripted-session gate. Dense
//! and MoE Paged students generate multi-char text on-device byte-identically.
//!
//! The demo ROM ([`gbf_kernel::asm_impl_shell::build_state_subword_demo_rom`])
//! pokes a host-encoded prompt as u16 token ids, warms the recurrent state,
//! samples from the paged head, feeds the full u16 id back through the
//! embedding lookup, and renders each token's literal multi-byte `id_bytes`.
//!
//! Gate (mirrors `demo`'s host byte-identity, extended to the subword surface):
//! (a) the on-device generated token-id sequence == the host mirror
//!     (`subword_host_generate`), byte-exact; and
//! (b) the rendered transcript BG bytes == the host id_bytes -> tile map of the
//!     decoded token stream (`expected_subword_transcript_bg`).

use gbf_data::bpe::BpeModel;
use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, Framebuffer,
    JoypadFrame, NormalizedTraceEvent, RunOutcome, TraceDropPolicy, TraceOrigin,
};
use gbf_foundation::sha256;
use gbf_hw::joypad::Button;
use gbf_kernel::asm_impl_shell::{
    BG_MAP_BASE, BG_MAP_STRIDE, KB_CELLS, KB_COLS, PROMPT_ROW, SHELL_PROMPT_CAP,
    SUBWORD_CURSOR_TILE, SUBWORD_KEY_BYTES, SUBWORD_NEWLINE_BYTE, SUBWORD_SPACE_BYTE,
    SubwordDemoRom, SubwordShellRom, TRANSCRIPT_CELLS, TRANSCRIPT_COLS, TRANSCRIPT_ROWS,
};
use gbf_kernel::asm_impl_state::{S_RNG_ADDR, S_SAMPLED_ADDR, S_SAMPLED_HI_ADDR};
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_from_candidates_trace};
use gbf_kernel::state_model_ref::IntStateLoweredModel;

use crate::one_token::OneTokenError;

/// Plan real button frames for typing ASCII bytes on the shell's 4x19
/// keyboard. `key_bytes[cell]` is the byte produced by A at that cell. Every
/// press is followed by a release because the ROM acts on newly-pressed edges.
/// Returns `None` when the key table is malformed or the prompt contains a byte
/// that the on-device keyboard cannot enter.
#[must_use]
pub fn subword_typing_script(prompt: &[u8], key_bytes: &[u8]) -> Option<Vec<JoypadFrame>> {
    if key_bytes.len() != usize::from(KB_CELLS) {
        return None;
    }
    let mut frames = Vec::new();
    let push_press = |frames: &mut Vec<JoypadFrame>, button: Button| {
        frames.push(JoypadFrame::pressed(button));
        frames.push(JoypadFrame::default());
    };
    let mut cursor = 0u8;
    for &byte in prompt {
        let target = u8::try_from(key_bytes.iter().position(|&key| key == byte)?).ok()?;
        let (cursor_row, cursor_col) = (cursor / KB_COLS, cursor % KB_COLS);
        let (target_row, target_col) = (target / KB_COLS, target % KB_COLS);
        for _ in 0..target_row.saturating_sub(cursor_row) {
            push_press(&mut frames, Button::Down);
        }
        for _ in 0..cursor_row.saturating_sub(target_row) {
            push_press(&mut frames, Button::Up);
        }
        for _ in 0..target_col.saturating_sub(cursor_col) {
            push_press(&mut frames, Button::Right);
        }
        for _ in 0..cursor_col.saturating_sub(target_col) {
            push_press(&mut frames, Button::Left);
        }
        push_press(&mut frames, Button::A);
        cursor = target;
    }
    Some(frames)
}

/// Build the demo's byte-indexed 8x8 font (tile == byte for `0..128`) from the
/// committed M0 runtime ASCII font. The newline byte gets a return-arrow glyph;
/// non-printable bytes are blank. Separate from the charset `tile == id` font.
#[must_use]
pub fn subword_font_tiles() -> Vec<u8> {
    gbf_codegen::compile_state_subword::subword_font_tiles()
}

/// Host mirror of one subword demo run: zero state, one forward pass per prompt
/// token id (no RNG draws), then paged-sample -> render -> feed the FULL id back
/// until `n_cap` tokens OR the 200-cell transcript fills (a rendered newline
/// byte advances the row). Returns the generated full token ids in order.
///
/// This is the exact ROM stop rule: the transcript-fill test is applied AFTER a
/// token renders (all of its bytes), mirroring `expected_subword_transcript_bg`.
#[must_use]
pub fn subword_host_generate(
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    id_bytes: &[Vec<u8>],
    prompt_ids: &[u16],
    rng_seed: u16,
    n_cap: u8,
) -> Vec<u16> {
    assert!(!prompt_ids.is_empty(), "demo ignores empty prompts");
    let mut rng = XorShift16::new(rng_seed);
    let mut state = lowered.zero_state();
    let mut trace = None;
    for &id in prompt_ids {
        trace = Some(lowered.forward_at(usize::from(id), &mut state));
    }
    let mut trace = trace.expect("prompt is nonempty");
    let mut sequence = Vec::new();
    let mut cell: usize = 0;
    loop {
        // Paged draw over the finalized top-k heap (== on-device `sample_paged`).
        let cands: Vec<(i32, usize)> = trace
            .topk_heap
            .iter()
            .take(usize::from(cfg.k()))
            .map(|e| (e.logit, e.id))
            .collect();
        let pick = sample_topk_from_candidates_trace(&cands, cfg.scale_q16(), &mut rng).picked;
        sequence.push(pick as u16);
        // advance the transcript cursor exactly like the render routine
        for &b in id_bytes.get(pick).map(Vec::as_slice).unwrap_or(&[]) {
            if cell >= usize::from(TRANSCRIPT_CELLS) {
                break;
            }
            if b == SUBWORD_NEWLINE_BYTE {
                cell = (cell / usize::from(TRANSCRIPT_COLS) + 1) * usize::from(TRANSCRIPT_COLS);
            } else {
                cell += 1;
            }
        }
        if sequence.len() >= usize::from(n_cap) || cell >= usize::from(TRANSCRIPT_CELLS) {
            break;
        }
        trace = lowered.forward_at(pick, &mut state);
    }
    sequence
}

/// Expected transcript BG cells after rendering `sequence` via `id_bytes`:
/// printable ASCII paints `tile == byte`, newline advances the row, and every
/// other decoded byte paints `?`. The block cursor sits at the next cell unless
/// the region filled.
/// Byte-exact mirror of the ROM `ui_render_bytes` + `demo` cursor semantics.
#[must_use]
pub fn expected_subword_transcript_bg(sequence: &[u16], id_bytes: &[Vec<u8>]) -> Vec<u8> {
    let cols = usize::from(TRANSCRIPT_COLS);
    let cells = usize::from(TRANSCRIPT_CELLS);
    let mut bg = vec![SUBWORD_SPACE_BYTE; cells];
    let mut cell = 0usize;
    let mut full = false;
    'outer: for &id in sequence {
        for &b in id_bytes
            .get(usize::from(id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if b == SUBWORD_NEWLINE_BYTE {
                cell = (cell / cols + 1) * cols;
            } else {
                bg[cell] = subword_display_tile(b);
                cell += 1;
            }
            if cell >= cells {
                full = true;
                break 'outer;
            }
        }
    }
    if !full {
        bg[cell] = SUBWORD_CURSOR_TILE;
    }
    bg
}

/// The decoded transcript text of a token-id sequence, mirroring
/// `BpeModel::decode` (concatenated literal bytes, lossy UTF-8), for evidence.
#[must_use]
pub fn decode_ids(sequence: &[u16], id_bytes: &[Vec<u8>]) -> String {
    let mut bytes = Vec::new();
    for &id in sequence {
        if let Some(b) = id_bytes.get(usize::from(id)) {
            bytes.extend_from_slice(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Map one decoded token byte onto the demo's byte-indexed display surface.
///
/// The model feedback path always keeps the real full token id; this function
/// only defines the visible glyph. Printable ASCII renders literally, newline
/// is handled by the caller as a row advance, and every other byte renders as
/// `?` so arbitrary byte-level BPE output never indexes an absent/blank glyph.
#[must_use]
pub const fn subword_display_tile(byte: u8) -> u8 {
    if byte.is_ascii_graphic() || byte == SUBWORD_SPACE_BYTE {
        byte
    } else {
        b'?'
    }
}

/// Result of one scripted subword demo session.
#[derive(Debug, Clone)]
pub struct SubwordSessionResult {
    /// Host-generated full token ids.
    pub host_sequence: Vec<u16>,
    /// Full u16 ids read from the device sampler at every token boundary.
    pub device_sequence: Vec<u16>,
    pub sequence_matches: bool,
    pub n_tokens: usize,
    /// (a) transcript BG bytes match the host id_bytes->tile render.
    pub transcript_bg_ok: bool,
    pub bg_first_mismatch: Option<(usize, u8, u8)>,
    /// The rendered transcript sha256 (on-device BG bytes).
    pub transcript_sha256: String,
    /// Final framebuffer after the run.
    pub framebuffer: Framebuffer,
    pub decoded_text: String,
    /// Exact emulator M-cycles for each prompt forward, including the demo
    /// boundary transition but excluding boot and host pokes.
    pub warmup_m_cycles: Vec<u64>,
    /// Exact emulator M-cycles for each generated token, including sampling
    /// and multi-byte rendering through the token boundary.
    pub generation_m_cycles: Vec<u64>,
}

fn emu_err(e: impl std::fmt::Display) -> OneTokenError {
    OneTokenError::Emulator(e.to_string())
}

fn bg_row_addr(row: u8) -> u16 {
    BG_MAP_BASE + u16::from(row) * BG_MAP_STRIDE
}

/// Drive one scripted subword demo session: boot, poke the host-encoded prompt
/// ids + RNG seed, set `go`, run warmup + `n_gen` token boundaries, then read
/// the full sampled u16 id at every boundary and the transcript BG. The result
/// exposes direct device-id parity plus byte-render parity against the host.
pub fn run_subword_demo_session(
    rom: &SubwordDemoRom,
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    id_bytes: &[Vec<u8>],
    prompt_ids: &[u16],
    rng_seed: u16,
) -> Result<SubwordSessionResult, OneTokenError> {
    assert!(!prompt_ids.is_empty(), "prompt must be nonempty");
    let host_sequence = subword_host_generate(
        lowered,
        cfg,
        id_bytes,
        prompt_ids,
        rng_seed,
        rom.n_gen_tokens,
    );

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(emu_err)?;

    let frame_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    // Per-token budget: the MoE-scaled MAC budget (wide for the slow MoE router),
    // matching the byte-exact MoE ROM gates, ×2 to cover the demo's extra
    // per-token UI work (transcript clear on gen start, multi-char render). The
    // render uses LCD-off VRAM batches (no VBlank spin), so this bounds an honest
    // slow forward; a genuine hang still spins past any finite budget.
    let token_budget = match crate::stateful::state_run_budget(lowered) {
        CycleBudget::Clock(c) => CycleBudget::Clock(gbf_emu::ClockCycles(c.0.saturating_mul(4))),
        other => other,
    };

    // Boot to idle FIRST — the ROM zeroes its shell control block (including
    // `prompt_len`) at boot, so all inputs are poked AFTER reaching the idle
    // loop. The prompt-id buffer lives above the zeroed block but is poked here
    // too for a single clean point of truth.
    match emu
        .run_fast_until_pc(rom.idle_pc, frame_budget)
        .map_err(emu_err)?
    {
        RunOutcome::TrapHit { .. } => {}
        other => {
            return Err(OneTokenError::Emulator(format!(
                "boot did not reach idle: {other:?}"
            )));
        }
    }
    for (i, &id) in prompt_ids.iter().enumerate() {
        let addr = rom.prompt_ids_addr + (2 * i) as u16;
        let b = id.to_le_bytes();
        emu.poke(addr, b[0]).map_err(emu_err)?;
        emu.poke(addr + 1, b[1]).map_err(emu_err)?;
    }
    emu.poke(rom.prompt_len_addr, prompt_ids.len() as u8)
        .map_err(emu_err)?;
    let seed = rng_seed.to_le_bytes();
    emu.poke(S_RNG_ADDR, seed[0]).map_err(emu_err)?;
    emu.poke(S_RNG_ADDR + 1, seed[1]).map_err(emu_err)?;
    emu.poke(rom.go_addr, 1).map_err(emu_err)?;

    let run_to = |emu: &mut Emulator, pc: u16, phase: &str| -> Result<(), OneTokenError> {
        emu.step().map_err(emu_err)?;
        match emu.run_fast_until_pc(pc, token_budget).map_err(emu_err)? {
            RunOutcome::TrapHit { .. } => Ok(()),
            other => Err(OneTokenError::Emulator(format!(
                "did not reach {phase} at {pc:#06x}: {other:?}"
            ))),
        }
    };

    // Warmup: one boundary per prompt id.
    let mut warmup_m_cycles = Vec::with_capacity(prompt_ids.len());
    for _ in 0..prompt_ids.len() {
        let start = emu.m_cycle_count_floor().0;
        run_to(&mut emu, rom.warm_boundary_pc, "warm boundary")?;
        warmup_m_cycles.push(emu.m_cycle_count_floor().0.saturating_sub(start));
    }
    // Generation: one boundary per token in the host mirror.
    let mut generation_m_cycles = Vec::with_capacity(host_sequence.len());
    let mut device_sequence = Vec::with_capacity(host_sequence.len());
    for _ in 0..host_sequence.len() {
        let start = emu.m_cycle_count_floor().0;
        run_to(&mut emu, rom.token_boundary_pc, "token boundary")?;
        generation_m_cycles.push(emu.m_cycle_count_floor().0.saturating_sub(start));
        let lo = emu.peek(S_SAMPLED_ADDR).map_err(emu_err)?;
        let hi = emu.peek(S_SAMPLED_HI_ADDR).map_err(emu_err)?;
        device_sequence.push(u16::from_le_bytes([lo, hi]));
    }
    run_to(&mut emu, rom.gen_done_pc, "generation done")?;

    // Read the transcript BG region.
    let cols = usize::from(TRANSCRIPT_COLS);
    let mut bg = Vec::with_capacity(usize::from(TRANSCRIPT_CELLS));
    for row in 0..TRANSCRIPT_ROWS {
        let r = emu.peek_range(bg_row_addr(row), cols).map_err(emu_err)?;
        bg.extend_from_slice(&r);
    }
    let expected = expected_subword_transcript_bg(&host_sequence, id_bytes);
    let bg_first_mismatch = bg
        .iter()
        .zip(expected.iter())
        .enumerate()
        .find(|(_, (a, e))| a != e)
        .map(|(i, (&a, &e))| (i, e, a));
    let transcript_bg_ok = bg_first_mismatch.is_none() && bg.len() == expected.len();

    let framebuffer = emu.framebuffer();
    Ok(SubwordSessionResult {
        n_tokens: host_sequence.len(),
        sequence_matches: device_sequence == host_sequence,
        transcript_sha256: sha256(&bg).to_hex(),
        transcript_bg_ok,
        bg_first_mismatch,
        framebuffer,
        decoded_text: decode_ids(&host_sequence, id_bytes),
        host_sequence,
        device_sequence,
        warmup_m_cycles,
        generation_m_cycles,
    })
}

/// Result of a fully interactive wide-vocabulary session. Unlike
/// [`run_subword_demo_session`], this path never pokes prompt IDs or a `go`
/// flag: every prompt byte enters through emulated JOYP frames, START invokes
/// the cartridge-resident BPE encoder, and the device token buffer is inspected
/// at the tokenizer boundary before recurrent warmup begins.
#[derive(Debug, Clone)]
pub struct TypedSubwordSessionResult {
    pub prompt: String,
    pub typing_frames: usize,
    /// Host-poke addresses observed by the emulator. Acceptance requires this
    /// to be empty: the RNG seed and all model controls are ROM-initialized.
    pub host_poke_addresses: Vec<u16>,
    pub rng_seed_initialized: bool,
    pub device_prompt_bytes: Vec<u8>,
    pub prompt_bytes_match: bool,
    pub prompt_echo_ok: bool,
    pub host_prompt_ids: Vec<u16>,
    pub device_prompt_ids: Vec<u16>,
    pub tokenization_matches: bool,
    pub tokenization_m_cycles: u64,
    pub returned_to_idle: bool,
    pub prompt_reset_ok: bool,
    pub generation: SubwordSessionResult,
}

impl TypedSubwordSessionResult {
    #[must_use]
    pub fn all_gates_pass(&self) -> bool {
        self.host_poke_addresses.is_empty()
            && self.rng_seed_initialized
            && self.prompt_bytes_match
            && self.prompt_echo_ok
            && self.tokenization_matches
            && self.generation.sequence_matches
            && self.generation.transcript_bg_ok
            && self.returned_to_idle
            && self.prompt_reset_ok
    }
}

fn read_u16_ids(emu: &Emulator, base: u16, len: usize) -> Result<Vec<u16>, OneTokenError> {
    let bytes = emu
        .peek_range(base, len.saturating_mul(2))
        .map_err(emu_err)?;
    Ok(bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

fn host_poke_addresses(events: Vec<NormalizedTraceEvent>) -> Vec<u16> {
    events
        .into_iter()
        .filter_map(|event| match event {
            NormalizedTraceEvent::MemoryWrite {
                addr,
                origin: TraceOrigin::HostPoke,
                ..
            } => Some(addr),
            _ => None,
        })
        .collect()
}

/// Drive the production-style playable subword cartridge end to end:
///
/// 1. boot to the keyboard and verify the ROM-baked RNG seed;
/// 2. type an ASCII prompt using only real press/release JOYP frames;
/// 3. verify both WRAM bytes and the visible prompt-row echo;
/// 4. press START, trap after on-device BPE, and compare its full u16 token
///    sequence exactly with [`BpeModel::encode`];
/// 5. warm up, generate, render, and compare every sampled u16 id with the host
///    integer mirror; and
/// 6. verify the shell clears the prompt and returns to keyboard idle.
///
/// The emulator host-poke audit is part of the returned evidence. This runner
/// intentionally calls neither `poke` nor `bus_write`; the cartridge must own
/// its RNG seed, prompt, tokenizer output, and submit control state.
pub fn run_typed_subword_shell_session(
    rom: &SubwordShellRom,
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    bpe: &BpeModel,
    id_bytes: &[Vec<u8>],
    prompt: &str,
) -> Result<TypedSubwordSessionResult, OneTokenError> {
    let prompt_bytes = prompt.as_bytes();
    assert!(
        prompt.is_ascii()
            && !prompt_bytes.is_empty()
            && prompt_bytes.len() <= usize::from(SHELL_PROMPT_CAP),
        "typed prompt must be nonempty ASCII and fit the one-row prompt cap"
    );
    assert_eq!(
        bpe.vocab_size(),
        lowered.topology.vocab,
        "tokenizer vocabulary must match the model"
    );
    let script = subword_typing_script(prompt_bytes, &SUBWORD_KEY_BYTES)
        .expect("every typed prompt byte must exist on the cartridge keyboard");
    let host_prompt_ids = bpe.encode(prompt);
    assert!(
        !host_prompt_ids.is_empty() && host_prompt_ids.len() <= 64,
        "encoded prompt must fit the cartridge's u16 token buffer"
    );
    let host_sequence = subword_host_generate(
        lowered,
        cfg,
        id_bytes,
        &host_prompt_ids,
        rom.rng_seed,
        rom.n_gen_tokens,
    );

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::DropOldest)
        .audit_host_pokes(true)
        .load_rom(&rom.rom)
        .map_err(emu_err)?;
    let frame_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    let token_budget = match crate::stateful::state_run_budget(lowered) {
        CycleBudget::Clock(c) => CycleBudget::Clock(gbf_emu::ClockCycles(c.0.saturating_mul(4))),
        other => other,
    };
    let run_to = |emu: &mut Emulator, pc: u16, phase: &str| -> Result<(), OneTokenError> {
        emu.step().map_err(emu_err)?;
        match emu.run_fast_until_pc(pc, token_budget).map_err(emu_err)? {
            RunOutcome::TrapHit { .. } => Ok(()),
            other => Err(OneTokenError::Emulator(format!(
                "did not reach {phase} at {pc:#06x}: {other:?}"
            ))),
        }
    };

    match emu
        .run_fast_until_pc(rom.idle_pc, frame_budget)
        .map_err(emu_err)?
    {
        RunOutcome::TrapHit { .. } => {}
        other => {
            return Err(OneTokenError::Emulator(format!(
                "boot did not reach typed-subword idle: {other:?}"
            )));
        }
    }
    let rng = [
        emu.peek(S_RNG_ADDR).map_err(emu_err)?,
        emu.peek(S_RNG_ADDR + 1).map_err(emu_err)?,
    ];
    let rng_seed_initialized = u16::from_le_bytes(rng) == rom.rng_seed;

    for frame in &script {
        emu.set_joypad(*frame);
        run_to(&mut emu, rom.idle_pc, "typed-subword idle frame")?;
    }
    emu.set_joypad(JoypadFrame::default());

    let device_prompt_len = usize::from(emu.peek(rom.prompt_byte_len_addr).map_err(emu_err)?);
    let device_prompt_bytes = emu
        .peek_range(rom.prompt_bytes_addr, device_prompt_len)
        .map_err(emu_err)?;
    let prompt_bytes_match = device_prompt_bytes == prompt_bytes;
    let mut expected_prompt_bg = vec![SUBWORD_SPACE_BYTE; usize::from(TRANSCRIPT_COLS)];
    expected_prompt_bg[..prompt_bytes.len()].copy_from_slice(prompt_bytes);
    let actual_prompt_bg = emu
        .peek_range(bg_row_addr(PROMPT_ROW), usize::from(TRANSCRIPT_COLS))
        .map_err(emu_err)?;
    let prompt_echo_ok = actual_prompt_bg == expected_prompt_bg;
    // Drain before model execution so the bounded trace cannot hide a prompt
    // or control poke behind the many subsequent ROM-bank switches.
    let mut poke_addresses = host_poke_addresses(emu.drain_trace());

    let tokenize_start = emu.m_cycle_count_floor().0;
    emu.set_joypad(JoypadFrame::pressed(Button::Start));
    run_to(&mut emu, rom.tokenize_done_pc, "on-device BPE completion")?;
    emu.set_joypad(JoypadFrame::default());
    let tokenization_m_cycles = emu.m_cycle_count_floor().0.saturating_sub(tokenize_start);
    let device_prompt_token_len =
        usize::from(emu.peek(rom.prompt_token_len_addr).map_err(emu_err)?);
    let device_prompt_ids = read_u16_ids(&emu, rom.prompt_ids_addr, device_prompt_token_len)?;
    let tokenization_matches = device_prompt_ids == host_prompt_ids;

    let mut warmup_m_cycles = Vec::with_capacity(device_prompt_ids.len());
    for _ in 0..device_prompt_ids.len() {
        let start = emu.m_cycle_count_floor().0;
        run_to(&mut emu, rom.warm_boundary_pc, "warm boundary")?;
        warmup_m_cycles.push(emu.m_cycle_count_floor().0.saturating_sub(start));
    }
    let mut generation_m_cycles = Vec::with_capacity(host_sequence.len());
    let mut device_sequence = Vec::with_capacity(host_sequence.len());
    for _ in 0..host_sequence.len() {
        let start = emu.m_cycle_count_floor().0;
        run_to(&mut emu, rom.token_boundary_pc, "token boundary")?;
        generation_m_cycles.push(emu.m_cycle_count_floor().0.saturating_sub(start));
        let lo = emu.peek(S_SAMPLED_ADDR).map_err(emu_err)?;
        let hi = emu.peek(S_SAMPLED_HI_ADDR).map_err(emu_err)?;
        device_sequence.push(u16::from_le_bytes([lo, hi]));
    }
    run_to(&mut emu, rom.gen_done_pc, "generation done")?;

    let cols = usize::from(TRANSCRIPT_COLS);
    let mut bg = Vec::with_capacity(usize::from(TRANSCRIPT_CELLS));
    for row in 0..TRANSCRIPT_ROWS {
        bg.extend_from_slice(&emu.peek_range(bg_row_addr(row), cols).map_err(emu_err)?);
    }
    let expected_bg = expected_subword_transcript_bg(&host_sequence, id_bytes);
    let bg_first_mismatch = bg
        .iter()
        .zip(expected_bg.iter())
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected)
        .map(|(index, (&actual, &expected))| (index, expected, actual));
    let transcript_bg_ok = bg_first_mismatch.is_none() && bg.len() == expected_bg.len();
    let framebuffer = emu.framebuffer();

    let returned_to_idle = {
        run_to(&mut emu, rom.idle_pc, "return to typed-subword idle")?;
        true
    };
    let prompt_len_after = emu.peek(rom.prompt_byte_len_addr).map_err(emu_err)?;
    let token_len_after = emu.peek(rom.prompt_token_len_addr).map_err(emu_err)?;
    let prompt_bg_after = emu
        .peek_range(bg_row_addr(PROMPT_ROW), usize::from(TRANSCRIPT_COLS))
        .map_err(emu_err)?;
    let prompt_reset_ok = prompt_len_after == 0
        && token_len_after == 0
        && prompt_bg_after
            .iter()
            .all(|&tile| tile == SUBWORD_SPACE_BYTE);
    poke_addresses.extend(host_poke_addresses(emu.drain_trace()));

    let generation = SubwordSessionResult {
        n_tokens: host_sequence.len(),
        sequence_matches: device_sequence == host_sequence,
        transcript_sha256: sha256(&bg).to_hex(),
        transcript_bg_ok,
        bg_first_mismatch,
        framebuffer,
        decoded_text: decode_ids(&host_sequence, id_bytes),
        host_sequence,
        device_sequence,
        warmup_m_cycles,
        generation_m_cycles,
    };
    Ok(TypedSubwordSessionResult {
        prompt: prompt.to_string(),
        typing_frames: script.len(),
        host_poke_addresses: poke_addresses,
        rng_seed_initialized,
        device_prompt_bytes,
        prompt_bytes_match,
        prompt_echo_ok,
        host_prompt_ids,
        device_prompt_ids,
        tokenization_matches,
        tokenization_m_cycles,
        returned_to_idle,
        prompt_reset_ok,
        generation,
    })
}

/// A synthetic printable `id_bytes` table for the always-on gate: byte-token
/// ids 0..=255 map to their own byte; merged ids >= 256 map to a short
/// deterministic printable string, so decoding is visible and a wrong id
/// renders visibly-wrong bytes. Every byte stays in the printable ASCII render
/// range (letters/space) so the transcript render is unambiguous.
#[must_use]
pub fn synthetic_id_bytes(vocab: usize) -> Vec<Vec<u8>> {
    let printable = |n: usize| -> u8 {
        // map to lowercase letters + space so the render font has a glyph
        let alphabet = b"abcdefghijklmnopqrstuvwxyz ";
        alphabet[n % alphabet.len()]
    };
    (0..vocab)
        .map(|id| {
            // Every synthetic token renders MULTIPLE chars so the multi-byte
            // render path is always exercised regardless of which ids the
            // (correct-but-arbitrary) synthetic sampler picks: a base pair for
            // ids < 256, a longer 2-3 char string for merged ids >= 256.
            if id < 256 {
                vec![printable(id), printable(id + 1)]
            } else {
                let n = id - 256;
                let len = 2 + (n % 2);
                (0..len).map(|k| printable(n + k)).collect()
            }
        })
        .collect()
}
