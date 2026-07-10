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

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, Framebuffer,
    RunOutcome, TraceDropPolicy,
};
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_shell::{
    BG_MAP_BASE, BG_MAP_STRIDE, SUBWORD_CURSOR_TILE, SUBWORD_FONT_BYTES, SUBWORD_NEWLINE_BYTE,
    SUBWORD_SPACE_BYTE, SubwordDemoRom, TRANSCRIPT_CELLS, TRANSCRIPT_COLS, TRANSCRIPT_ROWS,
};
use gbf_kernel::asm_impl_state::{S_RNG_ADDR, S_SAMPLED_ADDR, S_SAMPLED_HI_ADDR};
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_from_candidates_trace};
use gbf_kernel::state_model_ref::IntStateLoweredModel;

use crate::one_token::OneTokenError;

/// Build the demo's byte-indexed 8x8 font (tile == byte for `0..128`) from the
/// committed M0 runtime ASCII font. The newline byte gets a return-arrow glyph;
/// non-printable bytes are blank. Separate from the charset `tile == id` font.
#[must_use]
pub fn subword_font_tiles() -> Vec<u8> {
    // The kernel's demo shares the shell newline glyph (a return arrow).
    let newline_glyph = crate::shell::NEWLINE_GLYPH;
    let font = gbf_runtime::text::font_bytes();
    let mut out = Vec::with_capacity(SUBWORD_FONT_BYTES);
    for byte in 0..128u8 {
        if byte == SUBWORD_NEWLINE_BYTE {
            out.extend_from_slice(&newline_glyph);
        } else if (0x20..0x7F).contains(&byte) {
            let ascii = byte as usize;
            out.extend_from_slice(&font[ascii * 16..ascii * 16 + 16]);
        } else {
            out.extend_from_slice(&[0u8; 16]);
        }
    }
    out
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
/// each token's literal bytes paint `tile == byte` (a newline byte advances the
/// row), and the block cursor sits at the next cell unless the region filled.
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
                bg[cell] = b;
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
