//! Interactive generation shell bring-up (bd-1kbv1, v0 demo scope): the
//! playable ROM from `gbf_kernel::asm_impl_shell` driven end-to-end in the
//! emulator with injected joypad frames — typing a fixed prompt on the
//! on-screen charset-80 keyboard, submitting, and gating the resulting
//! sampled generation byte-exactly against the host integer evaluator.
//!
//! Gates (each numbered demo item is gated before the next):
//! 1. Boot + text rendering: after boot the BG map must hold the keyboard
//!    grid, status row, and cleared transcript (checked cell-by-cell).
//! 2. Prompt entry: after the scripted typing the prompt row must echo the
//!    typed charset ids exactly.
//! 3. Generation: the on-device sequence (output ring) must equal the host
//!    evaluator's — same prompt warmup, same RNG seed, same sampler — and
//!    the transcript BG region must contain exactly the rendered glyph
//!    tiles (newline advancing the row, block cursor at the final cell).
//! 4. Determinism: two full scripted sessions must produce identical
//!    sequences and identical framebuffer hashes at the fixed checkpoints
//!    (post-boot, post-typing, post-generation).
//!
//! Evidence (`interactive_shell_v0.v1`) is produced by the
//! `interactive-shell` bin — never hand-written.

use std::path::Path;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, Framebuffer,
    JoypadFrame, RunOutcome, TraceDropPolicy,
};
use gbf_foundation::sha256;
use gbf_hw::joypad::Button;
use gbf_kernel::asm_impl_shell::{
    BG_MAP_BASE, BG_MAP_STRIDE, KB_CELLS, KB_COLS, KB_ORIGIN_ROW, KB_ROWS, MSG_ROW, PROMPT_ROW,
    SHELL_CURSOR_TILE, SHELL_FONT_BYTES, SHELL_INVERT_TILE_OFFSET, SHELL_MSG_TEXT_IDS,
    SHELL_NEWLINE_ID, SHELL_PROMPT_CAP, SHELL_SPACE_ID, SHELL_STATUS_TEXT_IDS, STATUS_ROW,
    ShellRom, TRANSCRIPT_CELLS, TRANSCRIPT_COLS, TRANSCRIPT_ROWS, build_state_shell_rom,
};
use gbf_kernel::asm_impl_state::{S_OUT_BASE, S_RNG_ADDR};
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_trace};
use gbf_kernel::state_model_ref::{IntStateLoweredModel, STATE_SLOTS};
use serde::Serialize;

use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};
use crate::sampling::SamplerSettingFacts;
use crate::stateful::{
    StateCheckpointFacts, id_to_char, load_state_checkpoint, render_char_sample,
};

/// Committed arm-B checkpoint export (same as the stateful bring-up).
pub const SHELL_EXPORT_DIR: &str = crate::stateful::STATE_EXPORT_DIR;

/// The pinned demo decode setting (planv0 default): top-k 8 at T = 0.8.
pub const SHELL_TOP_K: u8 = 8;
pub const SHELL_TEMPERATURE: f64 = 0.8;

/// The fixed gate prompt.
pub const SHELL_GATE_PROMPT: &str = "The ";
/// The fixed gate RNG seed (poked at [`S_RNG_ADDR`] before boot).
pub const SHELL_GATE_RNG_SEED: u16 = 0x5EED;

/// Custom 8x8 glyph for the newline id (a return arrow); every other glyph
/// comes from the committed M0 runtime font asset.
pub const NEWLINE_GLYPH: [u8; 16] = [
    0x00, 0x00, 0x02, 0x02, 0x02, 0x02, 0x12, 0x12, 0x3E, 0x3E, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00,
];

/// Encode-side inverse of `stateful::id_to_char` over the 76 printables.
#[must_use]
pub fn char_to_id(c: char) -> Option<u8> {
    (0..76u8).find(|&id| id_to_char(id) == c)
}

/// Build the 76-glyph shell font from the committed M0 runtime font asset
/// (ASCII-indexed 8x8 tiles) plus the custom newline glyph.
#[must_use]
pub fn shell_font_tiles() -> Vec<u8> {
    let font = gbf_runtime::text::font_bytes();
    let mut out = Vec::with_capacity(SHELL_FONT_BYTES);
    for id in 0..76u8 {
        if id == SHELL_NEWLINE_ID {
            out.extend_from_slice(&NEWLINE_GLYPH);
        } else {
            let ascii = id_to_char(id) as usize;
            debug_assert!(ascii < 128);
            out.extend_from_slice(&font[ascii * 16..ascii * 16 + 16]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// typing script (injected joypad frames)
// ---------------------------------------------------------------------------

/// Plan the joypad frames that type `prompt_ids` on the 4x19 grid: for each
/// char, row moves then column moves then A, every press followed by a
/// release frame (the ROM edge-detects newly-pressed buttons). START is
/// injected separately by the session driver.
#[must_use]
pub fn typing_script(prompt_ids: &[u8]) -> Vec<JoypadFrame> {
    let mut frames = Vec::new();
    let push_press = |frames: &mut Vec<JoypadFrame>, b: Button| {
        frames.push(JoypadFrame::pressed(b));
        frames.push(JoypadFrame::default());
    };
    let mut cur: u8 = 0;
    for &id in prompt_ids {
        debug_assert!(id < KB_CELLS);
        let (cr, cc) = (cur / KB_COLS, cur % KB_COLS);
        let (tr, tc) = (id / KB_COLS, id % KB_COLS);
        for _ in 0..tr.saturating_sub(cr) {
            push_press(&mut frames, Button::Down);
        }
        for _ in 0..cr.saturating_sub(tr) {
            push_press(&mut frames, Button::Up);
        }
        for _ in 0..tc.saturating_sub(cc) {
            push_press(&mut frames, Button::Right);
        }
        for _ in 0..cc.saturating_sub(tc) {
            push_press(&mut frames, Button::Left);
        }
        push_press(&mut frames, Button::A);
        cur = id;
    }
    frames
}

// ---------------------------------------------------------------------------
// host mirror
// ---------------------------------------------------------------------------

/// Host mirror of one shell generation run: zero state, one forward pass
/// per prompt char (no RNG draws), then sample-render-feedback until
/// `n_cap` tokens or the 200-cell transcript region fills (newline advances
/// the row) — the exact ROM stop rule.
#[must_use]
pub fn shell_host_generate(
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    prompt_ids: &[u8],
    rng_seed: u16,
    n_cap: u8,
) -> Vec<u8> {
    assert!(!prompt_ids.is_empty(), "shell ignores empty submits");
    let mut rng = XorShift16::new(rng_seed);
    let mut state = [0i32; STATE_SLOTS];
    let mut trace = None;
    for &c in prompt_ids {
        trace = Some(lowered.forward(c, &mut state));
    }
    let mut trace = trace.expect("prompt is nonempty");
    let mut sequence = Vec::new();
    let mut cell: u16 = 0;
    loop {
        let pick = sample_topk_trace(&trace.logits, cfg, &mut rng).picked as u8;
        sequence.push(pick);
        if pick == SHELL_NEWLINE_ID {
            cell = (cell / u16::from(TRANSCRIPT_COLS) + 1) * u16::from(TRANSCRIPT_COLS);
        } else {
            cell += 1;
        }
        if sequence.len() >= usize::from(n_cap) || cell >= u16::from(TRANSCRIPT_CELLS) {
            break;
        }
        trace = lowered.forward(pick, &mut state);
    }
    sequence
}

/// Expected transcript BG cells after rendering `sequence` (mirrors
/// `ui_render_token`): glyph tiles, newline rows, and the block cursor at
/// the final cell unless the region filled.
#[must_use]
pub fn expected_transcript_bg(sequence: &[u8]) -> Vec<u8> {
    let cols = usize::from(TRANSCRIPT_COLS);
    let cells = usize::from(TRANSCRIPT_CELLS);
    let mut bg = vec![SHELL_SPACE_ID; cells];
    let mut cell = 0usize;
    for &t in sequence {
        if t == SHELL_NEWLINE_ID {
            cell = (cell / cols + 1) * cols;
        } else {
            bg[cell] = t;
            cell += 1;
        }
        if cell >= cells {
            return bg;
        }
    }
    bg[cell] = SHELL_CURSOR_TILE;
    bg
}

/// Expected keyboard-grid BG rows for a cursor at `cursor` (cell index ==
/// charset id == tile index; the cursor cell is inverted).
#[must_use]
pub fn expected_keyboard_bg(cursor: u8) -> Vec<Vec<u8>> {
    (0..KB_ROWS)
        .map(|r| {
            (0..KB_COLS)
                .map(|c| {
                    let id = r * KB_COLS + c;
                    if id == cursor {
                        id + SHELL_INVERT_TILE_OFFSET
                    } else {
                        id
                    }
                })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// scripted session
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BgMismatch {
    pub region: String,
    pub cell_index: usize,
    pub expected_tile: u8,
    pub actual_tile: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellSessionResult {
    pub prompt_ids: Vec<u8>,
    pub rng_seed: u16,
    pub typing_frames: usize,
    /// Gate 1: post-boot BG chrome (keyboard grid, status row, cleared
    /// transcript/prompt rows) is exactly as specified.
    pub boot_chrome_ok: bool,
    /// Gate 2: prompt row echoes the typed ids before submit.
    pub prompt_echo_ok: bool,
    /// Gate 3a: output ring byte-identical to the host evaluator.
    pub n_tokens_generated: usize,
    pub host_sequence_sha256: String,
    pub rom_sequence_sha256: String,
    pub sequences_match: bool,
    pub first_divergence_index: Option<usize>,
    /// Gate 3b: transcript BG region contains exactly the rendered tiles.
    pub transcript_bg_ok: bool,
    /// Post-run chrome: prompt/message rows cleared, keyboard + status
    /// intact, shell back at the idle input loop.
    pub post_run_chrome_ok: bool,
    pub returned_to_idle: bool,
    pub bg_mismatches: Vec<BgMismatch>,
    /// Framebuffer sha256 at the fixed checkpoints.
    pub fb_sha256_after_boot: String,
    pub fb_sha256_after_typing: String,
    pub fb_sha256_after_generation: String,
    /// M-cycle deltas between consecutive warmup boundaries and token
    /// boundaries (the real UI update cadence, VBlank waits included).
    pub warm_boundary_m_cycles: Vec<u64>,
    pub token_boundary_m_cycles: Vec<u64>,
    #[serde(skip)]
    pub rom_sequence: Vec<u8>,
    #[serde(skip)]
    pub framebuffers: Vec<(String, Framebuffer)>,
}

impl ShellSessionResult {
    #[must_use]
    pub fn all_gates_pass(&self) -> bool {
        self.boot_chrome_ok
            && self.prompt_echo_ok
            && self.sequences_match
            && self.transcript_bg_ok
            && self.post_run_chrome_ok
            && self.returned_to_idle
    }
}

fn emu_err(e: impl std::fmt::Display) -> OneTokenError {
    OneTokenError::Emulator(e.to_string())
}

/// Step off the current trap PC, then run until `pc` (the trap labels are
/// re-hit every loop iteration, and `run_fast_until_pc` returns immediately
/// when PC already equals the target).
fn step_run_to(
    emu: &mut Emulator,
    pc: u16,
    budget: CycleBudget,
    phase: &str,
) -> Result<(), OneTokenError> {
    emu.step().map_err(emu_err)?;
    match emu.run_fast_until_pc(pc, budget).map_err(emu_err)? {
        RunOutcome::TrapHit { .. } => Ok(()),
        other => Err(OneTokenError::Emulator(format!(
            "did not reach {phase} at {pc:#06x}: {other:?}"
        ))),
    }
}

fn bg_row_addr(row: u8) -> u16 {
    BG_MAP_BASE + u16::from(row) * BG_MAP_STRIDE
}

fn check_row(
    emu: &Emulator,
    region: &str,
    row: u8,
    expected: &[u8],
    mismatches: &mut Vec<BgMismatch>,
) -> Result<bool, OneTokenError> {
    let actual = emu
        .peek_range(bg_row_addr(row), expected.len())
        .map_err(emu_err)?;
    let mut ok = true;
    for (i, (&e, &a)) in expected.iter().zip(actual.iter()).enumerate() {
        if e != a {
            ok = false;
            mismatches.push(BgMismatch {
                region: format!("{region}/row{row}"),
                cell_index: i,
                expected_tile: e,
                actual_tile: a,
            });
        }
    }
    Ok(ok)
}

fn check_transcript(
    emu: &Emulator,
    expected: &[u8],
    mismatches: &mut Vec<BgMismatch>,
) -> Result<bool, OneTokenError> {
    let cols = usize::from(TRANSCRIPT_COLS);
    let mut ok = true;
    for row in 0..TRANSCRIPT_ROWS {
        let want = &expected[usize::from(row) * cols..(usize::from(row) + 1) * cols];
        ok &= check_row(emu, "transcript", row, want, mismatches)?;
    }
    Ok(ok)
}

fn fb_hash(fb: &Framebuffer) -> String {
    sha256(fb.as_bytes().as_slice()).to_hex()
}

/// Run two settle frames of the idle loop (no buttons) so the PPU has
/// rendered a complete frame from the current BG map before a framebuffer
/// checkpoint is captured. Deterministic: no input edges, no state change.
fn settle_frames(
    emu: &mut Emulator,
    idle_pc: u16,
    budget: CycleBudget,
) -> Result<(), OneTokenError> {
    emu.set_joypad(JoypadFrame::default());
    for _ in 0..2 {
        step_run_to(emu, idle_pc, budget, "settle frame")?;
    }
    Ok(())
}

/// Drive one full scripted session: boot -> type the prompt -> submit ->
/// warmup -> generation -> back to idle, gating every phase.
pub fn run_shell_session(
    rom: &ShellRom,
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    prompt_ids: &[u8],
    rng_seed: u16,
) -> Result<ShellSessionResult, OneTokenError> {
    assert!(
        !prompt_ids.is_empty() && prompt_ids.len() <= usize::from(SHELL_PROMPT_CAP),
        "prompt must fit the prompt row"
    );
    let host_sequence = shell_host_generate(lowered, cfg, prompt_ids, rng_seed, rom.n_gen_tokens);
    let n_expected = host_sequence.len();

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(emu_err)?;
    let seed_bytes = rng_seed.to_le_bytes();
    emu.poke(S_RNG_ADDR, seed_bytes[0]).map_err(emu_err)?;
    emu.poke(S_RNG_ADDR + 1, seed_bytes[1]).map_err(emu_err)?;

    let frame_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(600));
    let token_budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000));

    // --- gate 1: boot + chrome ---
    match emu
        .run_fast_until_pc(rom.idle_pc, frame_budget)
        .map_err(emu_err)?
    {
        RunOutcome::TrapHit { .. } => {}
        other => {
            return Err(OneTokenError::Emulator(format!(
                "boot did not reach the idle loop: {other:?}"
            )));
        }
    }
    let mut mismatches = Vec::new();
    let mut boot_chrome_ok = true;
    boot_chrome_ok &= check_transcript(
        &emu,
        &vec![SHELL_SPACE_ID; usize::from(TRANSCRIPT_CELLS)],
        &mut mismatches,
    )?;
    boot_chrome_ok &= check_row(
        &emu,
        "prompt",
        PROMPT_ROW,
        &vec![SHELL_SPACE_ID; usize::from(TRANSCRIPT_COLS)],
        &mut mismatches,
    )?;
    for (r, want) in expected_keyboard_bg(0).into_iter().enumerate() {
        boot_chrome_ok &= check_row(
            &emu,
            "keyboard",
            KB_ORIGIN_ROW + r as u8,
            &want,
            &mut mismatches,
        )?;
    }
    boot_chrome_ok &= check_row(
        &emu,
        "status",
        STATUS_ROW,
        &SHELL_STATUS_TEXT_IDS,
        &mut mismatches,
    )?;
    settle_frames(&mut emu, rom.idle_pc, frame_budget)?;
    let fb_boot = emu.framebuffer();

    // --- gate 2: prompt entry ---
    let script = typing_script(prompt_ids);
    for frame in &script {
        emu.set_joypad(*frame);
        step_run_to(&mut emu, rom.idle_pc, frame_budget, "idle frame")?;
    }
    let mut prompt_row_expect = vec![SHELL_SPACE_ID; usize::from(TRANSCRIPT_COLS)];
    prompt_row_expect[..prompt_ids.len()].copy_from_slice(prompt_ids);
    let prompt_echo_ok = check_row(
        &emu,
        "prompt-typed",
        PROMPT_ROW,
        &prompt_row_expect,
        &mut mismatches,
    )?;
    settle_frames(&mut emu, rom.idle_pc, frame_budget)?;
    let fb_typed = emu.framebuffer();

    // --- gate 3: submit + warmup + generation ---
    emu.set_joypad(JoypadFrame::pressed(Button::Start));
    let mut warm_deltas = Vec::with_capacity(prompt_ids.len());
    let mut prev_cycles = emu.m_cycle_count_floor().0;
    step_run_to(
        &mut emu,
        rom.warm_boundary_pc,
        token_budget,
        "warm boundary",
    )?;
    emu.set_joypad(JoypadFrame::default());
    let now = emu.m_cycle_count_floor().0;
    warm_deltas.push(now.saturating_sub(prev_cycles));
    prev_cycles = now;
    for _ in 1..prompt_ids.len() {
        step_run_to(
            &mut emu,
            rom.warm_boundary_pc,
            token_budget,
            "warm boundary",
        )?;
        let now = emu.m_cycle_count_floor().0;
        warm_deltas.push(now.saturating_sub(prev_cycles));
        prev_cycles = now;
    }
    let mut token_deltas = Vec::with_capacity(n_expected);
    for _ in 0..n_expected {
        step_run_to(
            &mut emu,
            rom.token_boundary_pc,
            token_budget,
            "token boundary",
        )?;
        let now = emu.m_cycle_count_floor().0;
        token_deltas.push(now.saturating_sub(prev_cycles));
        prev_cycles = now;
    }
    step_run_to(&mut emu, rom.gen_done_pc, token_budget, "generation done")?;

    let rom_sequence = emu.peek_range(S_OUT_BASE, n_expected).map_err(emu_err)?;
    let first_divergence_index = host_sequence
        .iter()
        .zip(rom_sequence.iter())
        .position(|(h, r)| h != r);
    let sequences_match =
        first_divergence_index.is_none() && host_sequence.len() == rom_sequence.len();

    let transcript_bg_ok = check_transcript(
        &emu,
        &expected_transcript_bg(&host_sequence),
        &mut mismatches,
    )?;

    // --- post-run chrome + return to idle ---
    let returned_to_idle = {
        step_run_to(&mut emu, rom.idle_pc, frame_budget, "return to idle")?;
        true
    };
    let mut post_run_chrome_ok = true;
    post_run_chrome_ok &= check_row(
        &emu,
        "prompt-cleared",
        PROMPT_ROW,
        &vec![SHELL_SPACE_ID; usize::from(TRANSCRIPT_COLS)],
        &mut mismatches,
    )?;
    post_run_chrome_ok &= check_row(
        &emu,
        "msg-cleared",
        MSG_ROW,
        &vec![SHELL_SPACE_ID; SHELL_MSG_TEXT_IDS.len()],
        &mut mismatches,
    )?;
    let final_cursor = *prompt_ids.last().expect("prompt is nonempty");
    for (r, want) in expected_keyboard_bg(final_cursor).into_iter().enumerate() {
        post_run_chrome_ok &= check_row(
            &emu,
            "keyboard-after",
            KB_ORIGIN_ROW + r as u8,
            &want,
            &mut mismatches,
        )?;
    }
    post_run_chrome_ok &= check_row(
        &emu,
        "status-after",
        STATUS_ROW,
        &SHELL_STATUS_TEXT_IDS,
        &mut mismatches,
    )?;
    settle_frames(&mut emu, rom.idle_pc, frame_budget)?;
    let fb_done = emu.framebuffer();

    Ok(ShellSessionResult {
        prompt_ids: prompt_ids.to_vec(),
        rng_seed,
        typing_frames: script.len(),
        boot_chrome_ok,
        prompt_echo_ok,
        n_tokens_generated: rom_sequence.len(),
        host_sequence_sha256: sha256(&host_sequence).to_hex(),
        rom_sequence_sha256: sha256(&rom_sequence).to_hex(),
        sequences_match,
        first_divergence_index,
        transcript_bg_ok,
        post_run_chrome_ok,
        returned_to_idle,
        bg_mismatches: mismatches,
        fb_sha256_after_boot: fb_hash(&fb_boot),
        fb_sha256_after_typing: fb_hash(&fb_typed),
        fb_sha256_after_generation: fb_hash(&fb_done),
        warm_boundary_m_cycles: warm_deltas,
        token_boundary_m_cycles: token_deltas,
        rom_sequence,
        framebuffers: vec![
            ("screenshot_1_boot.pgm".to_string(), fb_boot),
            ("screenshot_2_prompt_typed.pgm".to_string(), fb_typed),
            ("screenshot_3_generation_done.pgm".to_string(), fb_done),
        ],
    })
}

/// Render a framebuffer as a binary PGM (P5) image, DMG shade 0 lightest.
#[must_use]
pub fn framebuffer_to_pgm(fb: &Framebuffer) -> Vec<u8> {
    let mut out = format!("P5\n{} {}\n255\n", Framebuffer::WIDTH, Framebuffer::HEIGHT).into_bytes();
    out.extend(fb.as_bytes().iter().map(|&v| 255 - v.min(3) * 85));
    out
}

// ---------------------------------------------------------------------------
// evidence report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ShellRomFacts {
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub ui_bank_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
    pub n_gen_tokens: u8,
    pub idle_pc: u16,
    pub warm_boundary_pc: u16,
    pub token_boundary_pc: u16,
    pub gen_done_pc: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellKeyboardFacts {
    pub layout: String,
    pub controls: String,
    pub planv0_divergences: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellCadenceFacts {
    pub mean_m_cycles_per_token_boundary: u64,
    pub seconds_per_token_dmg: f64,
    pub mean_m_cycles_per_warmup_char: u64,
    pub warmup_seconds_per_prompt_char_dmg: f64,
    pub idle_input_poll: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellDeterminismFacts {
    pub sessions: usize,
    pub sequences_identical: bool,
    pub framebuffer_hashes_identical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractiveShellReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub upstream_beads: Vec<&'static str>,
    pub git_sha: String,
    pub checkpoint: StateCheckpointFacts,
    pub sampler: SamplerSettingFacts,
    pub rom: ShellRomFacts,
    pub keyboard: ShellKeyboardFacts,
    pub prompt_text: String,
    pub session: ShellSessionResult,
    pub cadence: ShellCadenceFacts,
    pub determinism: ShellDeterminismFacts,
    pub interrupt_policy: String,
    pub caveats: Vec<String>,
}

/// Build the real-checkpoint shell ROM, run the scripted session twice
/// (determinism), and assemble the evidence report.
pub fn run_shell_bringup(
    repo_root: &Path,
    export_dir_rel: &str,
    n_gen_tokens: u8,
) -> Result<InteractiveShellReport, OneTokenError> {
    let export_dir = repo_root.join(export_dir_rel);
    let bundle = load_state_checkpoint(&export_dir)?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(SHELL_TOP_K, step, SHELL_TEMPERATURE)
        .map_err(|e| OneTokenError::Model(format!("sampler config: {e}")))?;
    let font = shell_font_tiles();
    let rom = build_state_shell_rom(&lowered, &cfg, n_gen_tokens, &font)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;

    let prompt_ids: Vec<u8> = SHELL_GATE_PROMPT
        .chars()
        .map(|c| char_to_id(c).expect("gate prompt chars are charset_v1 printables"))
        .collect();

    let session = run_shell_session(&rom, &lowered, &cfg, &prompt_ids, SHELL_GATE_RNG_SEED)?;
    let rerun = run_shell_session(&rom, &lowered, &cfg, &prompt_ids, SHELL_GATE_RNG_SEED)?;
    let determinism = ShellDeterminismFacts {
        sessions: 2,
        sequences_identical: session.rom_sequence == rerun.rom_sequence,
        framebuffer_hashes_identical: session.fb_sha256_after_boot == rerun.fb_sha256_after_boot
            && session.fb_sha256_after_typing == rerun.fb_sha256_after_typing
            && session.fb_sha256_after_generation == rerun.fb_sha256_after_generation,
    };

    let mean = |v: &[u64]| -> u64 {
        if v.is_empty() {
            0
        } else {
            v.iter().sum::<u64>() / v.len() as u64
        }
    };
    let mean_token = mean(&session.token_boundary_m_cycles);
    let mean_warm = mean(&session.warm_boundary_m_cycles);

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(InteractiveShellReport {
        schema: "interactive_shell_v0.v1",
        bead: "bd-1kbv1",
        upstream_beads: vec!["bd-59qiq", "bd-x5l2s", "bd-2mjkd", "bd-29ai4"],
        git_sha,
        checkpoint: StateCheckpointFacts {
            export_dir: export_dir_rel.to_string(),
            manifest_schema: bundle.manifest_schema,
            manifest_sha256: bundle.manifest_sha256,
            trainer_git_sha: bundle.manifest_git_sha,
            tensors_verified_sha256: bundle.tensors_verified,
        },
        sampler: SamplerSettingFacts {
            top_k: cfg.k(),
            scale_q16: cfg.scale_q16(),
            requested_temperature: SHELL_TEMPERATURE,
            effective_temperature: cfg.effective_temperature(step),
        },
        rom: ShellRomFacts {
            rom_bytes: rom.rom.len(),
            bank_count: rom.bank_count,
            driver_bytes: rom.driver_bytes,
            ui_bank_bytes: rom.ui_bank_bytes,
            weight_code_bytes: rom.weight_code_bytes,
            weight_chunk_count: rom.weight_chunk_count,
            table_bytes: rom.table_bytes,
            n_gen_tokens: rom.n_gen_tokens,
            idle_pc: rom.idle_pc,
            warm_boundary_pc: rom.warm_boundary_pc,
            token_boundary_pc: rom.token_boundary_pc,
            gen_done_pc: rom.gen_done_pc,
        },
        keyboard: ShellKeyboardFacts {
            layout: format!(
                "single-page {KB_ROWS}x{KB_COLS} grid holding all 76 charset_v1 ids in id \
                 order (cell index == charset id == tile index); cursor rendered as the \
                 inverted glyph; prompt cap {SHELL_PROMPT_CAP} chars (one BG row)"
            ),
            controls: "D-pad moves the cursor (clamped), A types, B backspaces, START submits"
                .to_string(),
            planv0_divergences: vec![
                "single page instead of the InteractionBundle three-page \
                 lowercase/uppercase/symbols keyboard (all 76 ids are directly reachable, \
                 so pages are unnecessary at this vocabulary size)"
                    .to_string(),
                "B is backspace, not the sketched one-shot shift (no shift is needed on a \
                 single page)"
                    .to_string(),
                "no page-cycling key; SELECT is unbound".to_string(),
            ],
        },
        prompt_text: SHELL_GATE_PROMPT.to_string(),
        cadence: ShellCadenceFacts {
            mean_m_cycles_per_token_boundary: mean_token,
            seconds_per_token_dmg: mean_token as f64 / DMG_M_CYCLES_PER_SECOND as f64,
            mean_m_cycles_per_warmup_char: mean_warm,
            warmup_seconds_per_prompt_char_dmg: mean_warm as f64 / DMG_M_CYCLES_PER_SECOND as f64,
            idle_input_poll: "joypad polled once per frame (59.7 Hz) in the idle loop; \
                              cursor/typing feedback lands in the same frame's VBlank"
                .to_string(),
        },
        session,
        determinism,
        interrupt_policy: "IME stays off for the whole session (di at entry, never ei): V3 \
                           weight chunks repurpose SP as the pop-stream pointer, so an ISR \
                           mid-kernel would corrupt the weight stream. UI work happens only at \
                           token boundaries and in the idle input loop; VBlank is found by \
                           polling LY. DeadlineAware yield integration is the planv0 scheduler \
                           beads' follow-up."
            .to_string(),
        caveats: vec![
            format!(
                "v0 demo shell, not the full M5 cooperative scheduler: the screen is static \
                 for one full forward pass between token boundaries (~{:.2} s of DMG time \
                 per token at this checkpoint); the per-token transcript glyph + block \
                 cursor and the per-char warmup highlight are the progress affordances.",
                mean_token as f64 / DMG_M_CYCLES_PER_SECOND as f64
            ),
            "The scripted gate runs in the emulator (gbf-emu headless adapter), not on \
             hardware; all timing is DMG M-cycle-accurate emulated time."
                .to_string(),
            "The RNG seed is host-poked for the gate; interactively, the XorShift16 state \
             carries across runs within a session (seed 0 canonicalizes to 1), so an \
             unpoked cart always plays the same first generation for a given prompt."
                .to_string(),
            "Sample quality must be judged honestly: the model is the 4-block d64 S5 arm-B \
             bring-up checkpoint (~3.3 bpc), so text is English-like at best."
                .to_string(),
        ],
    })
}

/// Render the report README (generated, not hand-written).
#[must_use]
pub fn shell_report_to_markdown(report: &InteractiveShellReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Interactive generation shell v0 ({})", report.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "A playable demo ROM: boot to an on-screen charset-80 keyboard, type a prompt with \
         the joypad, press START, and watch the stateful LinearState checkpoint generate \
         sampled text into an on-screen transcript — all on-device. This is the **v0 demo \
         shell** scope of bead {} (not the full M5 cooperative scheduler). Generated by \
         `cargo run -p gbf-bench --bin interactive-shell`; every number below is program \
         output at git `{}`.",
        report.bead, report.git_sha
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Checkpoint and decode");
    let _ = writeln!(out);
    let c = &report.checkpoint;
    let _ = writeln!(
        out,
        "- `{}` ({}), manifest sha256 `{}`, trainer git `{}`, {} tensors sha256-verified",
        c.export_dir,
        c.manifest_schema,
        &c.manifest_sha256[..16],
        c.trainer_git_sha,
        c.tensors_verified_sha256
    );
    let s = &report.sampler;
    let _ = writeln!(
        out,
        "- Sampler: top-k {}, requested T {} (scale_q16 {}, effective T {:.4}) — the pinned \
         integer exp-LUT + XorShift16 decode (bd-2mjkd)",
        s.top_k, s.requested_temperature, s.scale_q16, s.effective_temperature
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## ROM");
    let _ = writeln!(out);
    let r = &report.rom;
    let _ = writeln!(
        out,
        "- {} bytes ({} banks): bank-0 driver {} B, UI bank {} B (font + polled-joypad \
         keyboard + VBlank BG writer), weight code {} B in {} chunks, tables {} B",
        r.rom_bytes,
        r.bank_count,
        r.driver_bytes,
        r.ui_bank_bytes,
        r.weight_code_bytes,
        r.weight_chunk_count,
        r.table_bytes
    );
    let _ = writeln!(
        out,
        "- Screen: 10x20 transcript (rows 0-9), prompt row 11, message row 12, 4x19 \
         keyboard rows 13-16, help row 17; tile index == charset id, +128 = inverted"
    );
    let _ = writeln!(out, "- Keyboard: {}", report.keyboard.layout);
    let _ = writeln!(out, "- Controls: {}", report.keyboard.controls);
    let _ = writeln!(out);
    let _ = writeln!(out, "## planv0 InteractionBundle divergences (v0)");
    let _ = writeln!(out);
    for d in &report.keyboard.planv0_divergences {
        let _ = writeln!(out, "- {d}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Scripted gate");
    let _ = writeln!(out);
    let g = &report.session;
    let _ = writeln!(
        out,
        "Prompt `{}` (ids {:?}) typed via {} injected joypad frames, RNG seed 0x{:04X}, \
         then START:",
        report.prompt_text.escape_default(),
        g.prompt_ids,
        g.typing_frames,
        g.rng_seed
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| gate | result |\n|---|---|\n| 1. boot chrome (keyboard grid, status row, cleared \
         transcript) | {} |\n| 2. prompt row echoes typed ids | {} |\n| 3a. {} generated \
         ids byte-identical to host evaluator | {} |\n| 3b. transcript BG region contains \
         exactly the rendered tiles | {} |\n| post-run chrome + return to input | {} |\n| \
         4. determinism across {} sessions (sequences + framebuffer hashes) | {} |",
        pass(g.boot_chrome_ok),
        pass(g.prompt_echo_ok),
        g.n_tokens_generated,
        pass(g.sequences_match),
        pass(g.transcript_bg_ok),
        pass(g.post_run_chrome_ok && g.returned_to_idle),
        report.determinism.sessions,
        pass(
            report.determinism.sequences_identical
                && report.determinism.framebuffer_hashes_identical
        ),
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Sequence sha256: host `{}`, ROM `{}`",
        &g.host_sequence_sha256[..16],
        &g.rom_sequence_sha256[..16]
    );
    let _ = writeln!(
        out,
        "- Framebuffer sha256: boot `{}`, typed `{}`, done `{}` (screenshots committed as \
         PGM)",
        &g.fb_sha256_after_boot[..16],
        &g.fb_sha256_after_typing[..16],
        &g.fb_sha256_after_generation[..16]
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## UI cadence (the honest numbers)");
    let _ = writeln!(out);
    let mean = |v: &[u64]| -> u64 {
        if v.is_empty() {
            0
        } else {
            v.iter().sum::<u64>() / v.len() as u64
        }
    };
    let mt = mean(&g.token_boundary_m_cycles);
    let mw = mean(&g.warm_boundary_m_cycles);
    let _ = writeln!(
        out,
        "- Idle input: joypad polled once per frame (59.7 Hz), cursor/typing feedback \
         within one frame"
    );
    let _ = writeln!(
        out,
        "- Warmup: **{:.2} s per prompt char** on DMG ({} M-cycles mean); the consumed \
         prompt char is highlighted at each boundary",
        mw as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        mw
    );
    let _ = writeln!(
        out,
        "- Generation: **{:.2} s per token** on DMG ({} M-cycles mean, VBlank alignment \
         included) — one transcript glyph + block cursor per boundary; the screen is \
         otherwise static during the forward pass",
        mt as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        mt
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Interrupt policy");
    let _ = writeln!(out);
    let _ = writeln!(out, "- {}", report.interrupt_policy);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Caveats");
    let _ = writeln!(out);
    for c in &report.caveats {
        let _ = writeln!(out, "- {c}");
    }
    out
}

fn pass(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

/// Program-generated transcript text file contents.
#[must_use]
pub fn transcript_text(prompt_ids: &[u8], sequence: &[u8]) -> String {
    format!(
        "prompt: {}\n--- generated ({} tokens) ---\n{}\n",
        render_char_sample(prompt_ids),
        sequence.len(),
        render_char_sample(sequence)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_kernel::asm_impl_shell::synthetic_font_tiles;
    use gbf_kernel::state_model_ref::synthetic_state_checkpoint;

    #[test]
    fn char_to_id_inverts_id_to_char() {
        for id in 0..76u8 {
            assert_eq!(char_to_id(id_to_char(id)), Some(id));
        }
        assert_eq!(char_to_id('~'), None);
    }

    #[test]
    fn status_and_msg_ids_decode_to_intended_text() {
        let status: String = SHELL_STATUS_TEXT_IDS
            .iter()
            .map(|&i| id_to_char(i))
            .collect();
        assert_eq!(status, "A:KEY B:DEL ST:GO");
        let msg: String = SHELL_MSG_TEXT_IDS.iter().map(|&i| id_to_char(i)).collect();
        assert_eq!(msg, "GENERATING");
    }

    #[test]
    fn shell_font_uses_committed_runtime_font() {
        let font = shell_font_tiles();
        assert_eq!(font.len(), SHELL_FONT_BYTES);
        // space is blank, 'A' (id 0) matches the runtime asset
        let space = usize::from(SHELL_SPACE_ID) * 16;
        assert!(font[space..space + 16].iter().all(|&b| b == 0));
        assert_eq!(
            &font[0..16],
            &gbf_runtime::text::font_bytes()[usize::from(b'A') * 16..usize::from(b'A') * 16 + 16]
        );
        // newline glyph is the custom arrow
        let nl = usize::from(SHELL_NEWLINE_ID) * 16;
        assert_eq!(&font[nl..nl + 16], &NEWLINE_GLYPH);
    }

    #[test]
    fn typing_script_plans_grid_moves_with_release_frames() {
        // 'T' is id 19 = row 1 col 0: one Down press from the origin, then A.
        let script = typing_script(&[19]);
        assert_eq!(script.len(), 4);
        assert!(script[0].is_pressed(Button::Down));
        assert_eq!(script[1], JoypadFrame::default());
        assert!(script[2].is_pressed(Button::A));
        assert_eq!(script[3], JoypadFrame::default());
        // id 0 from the origin is just A.
        assert_eq!(typing_script(&[0]).len(), 2);
    }

    #[test]
    fn expected_transcript_handles_newlines_and_cursor() {
        let bg = expected_transcript_bg(&[0, 1, SHELL_NEWLINE_ID, 2]);
        assert_eq!(bg[0], 0);
        assert_eq!(bg[1], 1);
        assert_eq!(bg[2], SHELL_SPACE_ID);
        assert_eq!(bg[20], 2);
        assert_eq!(bg[21], SHELL_CURSOR_TILE);
        assert!(bg[22..].iter().all(|&t| t == SHELL_SPACE_ID));
    }

    /// Full-stack smoke on a synthetic checkpoint: boot, type 'A' (id 0),
    /// START, generate 3 tokens — every gate must pass and the on-device
    /// sequence must be byte-identical to the host mirror. This is the same
    /// machinery as the real-checkpoint evidence run.
    #[test]
    fn shell_session_matches_host_on_synthetic_model() {
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = SamplerConfig::new(8, 2253).expect("valid sampler");
        let font = synthetic_font_tiles();
        let rom = build_state_shell_rom(&lowered, &cfg, 3, &font).expect("builds");
        let result = run_shell_session(&rom, &lowered, &cfg, &[0], 0xBEEF).expect("runs");
        assert!(
            result.all_gates_pass(),
            "gate failures: boot_chrome={} prompt_echo={} seq={} (div {:?}) transcript={} \
             post={} idle={} mismatches={:?}",
            result.boot_chrome_ok,
            result.prompt_echo_ok,
            result.sequences_match,
            result.first_divergence_index,
            result.transcript_bg_ok,
            result.post_run_chrome_ok,
            result.returned_to_idle,
            result.bg_mismatches
        );
        assert_eq!(result.n_tokens_generated, 3);
        assert_eq!(result.warm_boundary_m_cycles.len(), 1);
        assert_eq!(result.token_boundary_m_cycles.len(), 3);
        assert!(result.token_boundary_m_cycles.iter().all(|&c| c > 0));
    }
}
