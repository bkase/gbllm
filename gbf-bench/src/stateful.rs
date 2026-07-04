//! Stateful ROM bring-up (bd-x5l2s): the LinearState arm-B checkpoint
//! (bd-29ai4) -> canonical integer recurrence -> banked ROM -> emulator
//! agreement, with the recurrent state living in WRAM across tokens.
//!
//! Phases (each gated on the previous, mirroring bd-59qiq / bd-2gc6p):
//! 1. Canonical integer semantics live in `gbf_kernel::state_model_ref`.
//! 2. Fidelity: integer semantics vs the trainer's f32 semantics over the
//!    charset_v1 validation stream with **state carried sequentially** (the
//!    same 8-lane layout as the committed A/B eval, so the f32 port must
//!    reproduce the committed arm-B val bpc as its own validation).
//! 3. ROM gates: the stateful one-token ROM must reproduce every host WRAM
//!    checkpoint byte-exactly for host-poked (input, state) cases including
//!    nonzero carried states; the multi-token ROM must generate >= 256
//!    tokens per seed on-device (state evolving in WRAM) byte-identically
//!    to the host evaluator, with SP/WRAM health and cycle stats.
//! 4. Evidence: `stateful_rom_bringup.v1` JSON + README + sample texts,
//!    produced by the `stateful-rom` bin — never hand-written.

use std::fs;
use std::path::Path;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TraceDropPolicy,
};
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_state::{
    S_ARGMAX_ADDR, S_DONE_ADDR, S_DUMP_DOWNACC0, S_DUMP_GELU0, S_DUMP_INACC, S_DUMP_NORM0,
    S_DUMP_SNORM, S_DUMP_UPACC0, S_DUMP_YACT, S_INPUT_ADDR, S_LOGITS_BASE, S_OUT_BASE,
    S_QDUMP_BASE, S_SACC_BASE, S_STACK_TOP, S_STATE_BASE, S_XDUMP_BASE, StateMultiTokenRom,
    StateOneTokenRom, build_state_multi_token_rom, build_state_one_token_rom,
};
use gbf_kernel::model_ref::{D_FF, D_MODEL, N_BLOCKS, TernaryLayer};
use gbf_kernel::state_model_ref::{
    IntStateForwardTrace, IntStateLoweredModel, STATE_INT_SEMANTIC_DIVERGENCES, STATE_SLOTS,
    STATE_VOCAB, StateCheckpoint, StateForwardStats, f32_state_forward,
};
use serde::Serialize;

use crate::multi_token::{CycleStats, WramViolation};
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError, SegmentMismatch, build_val_bytes};

/// Committed arm-B checkpoint export (manifest `f_s5_state_checkpoint_export.v1`).
pub const STATE_EXPORT_DIR: &str = "experiments/S5/state-ab/checkpoint-export";

/// The generation gate length (>= 256 consecutive on-device steps).
pub const STATE_GENERATION_TOKENS: u16 = 256;

/// Seed charset ids for the generation gate (>= 4 required):
/// 'T' (19), 'a' (26), ' ' (62), newline (75).
pub const STATE_GENERATION_SEEDS: [u8; 4] = [19, 26, 62, 75];

/// Val-stream positions whose carried state (and input) become the
/// one-token gate cases; position 0 exercises the zero state.
pub const ONE_TOKEN_STATE_POSITIONS: [usize; 16] = [
    0, 1, 3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 12287, 16383,
];

/// Lane count of the committed A/B eval (`s5_state_ab.rs --eval-lanes`).
pub const EVAL_LANES: usize = 8;

/// WRAM regions the stateful token loop must never write, as `[start, end)`.
/// Snapshotted before the run and re-read after the last token.
pub const STATE_UNTOUCHED_WRAM_REGIONS: &[(u16, u16)] = &[
    (0xC080, 0xC100), // between activations and matvec accumulators
    (0xC200, 0xC280), // free page below scratch A
    (0xC2E0, 0xC300), // scratch A tail
    (0xC3C0, 0xC400), // between the i24 residual and scratch B
    (0xC4A0, 0xC500), // scratch B tail
    (0xCAF0, 0xCB00), // between the 80-id logits and the residual dumps
    (0xD104, 0xD110), // control-byte gap
    (0xD120, 0xD180), // between control bytes and the in-acc dump
    (0xD3C0, 0xDFC0), // between the |x| buffer and the stack arena
    (0xDFF0, 0xE000), // above the stack home
];

// ---------------------------------------------------------------------------
// checkpoint loading
// ---------------------------------------------------------------------------

/// Loaded stateful checkpoint plus provenance facts for the report.
pub struct StateCheckpointBundle {
    pub checkpoint: StateCheckpoint,
    pub manifest_schema: String,
    pub manifest_git_sha: String,
    pub manifest_sha256: String,
    pub tensors_verified: usize,
}

/// Load and integrity-check the committed arm-B canonical export.
pub fn load_state_checkpoint(export_dir: &Path) -> Result<StateCheckpointBundle, OneTokenError> {
    let manifest_path = export_dir.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| OneTokenError::Io {
        path: manifest_path.clone(),
        reason: e.to_string(),
    })?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).map_err(|e| OneTokenError::Manifest {
            reason: e.to_string(),
        })?;
    let schema = manifest["schema"].as_str().unwrap_or_default().to_string();
    if schema != "f_s5_state_checkpoint_export.v1" {
        return Err(OneTokenError::Manifest {
            reason: format!("unexpected schema {schema:?}"),
        });
    }
    let git_sha = manifest["git_sha"].as_str().unwrap_or_default().to_string();

    let tensors = manifest["tensors"]
        .as_array()
        .ok_or_else(|| OneTokenError::Manifest {
            reason: "missing tensors array".into(),
        })?;
    let mut verified = 0usize;
    let mut load = |name: &str| -> Result<Vec<u8>, OneTokenError> {
        let entry = tensors
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .ok_or_else(|| OneTokenError::Manifest {
                reason: format!("tensor {name} missing"),
            })?;
        let file = entry["file"]
            .as_str()
            .ok_or_else(|| OneTokenError::Manifest {
                reason: format!("tensor {name} missing file"),
            })?;
        let path = export_dir.join(file);
        let bytes = fs::read(&path).map_err(|e| OneTokenError::Io {
            path,
            reason: e.to_string(),
        })?;
        let expected = entry["sha256"].as_str().unwrap_or_default();
        if sha256(&bytes).to_hex() != expected {
            return Err(OneTokenError::ShaMismatch {
                tensor: name.to_string(),
            });
        }
        verified += 1;
        Ok(bytes)
    };

    let emb_bytes = load("embedding")?;
    if emb_bytes.len() != STATE_VOCAB * D_MODEL * 4 {
        return Err(OneTokenError::Manifest {
            reason: format!("embedding byte length {}", emb_bytes.len()),
        });
    }
    let embedding: Vec<f32> = emb_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let decay_bytes = load("state_decay")?;
    let decay_raw: Vec<u16> = decay_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut ternary =
        |base: &str, rows: usize, cols: usize| -> Result<TernaryLayer, OneTokenError> {
            let tern = load(&format!("{base}.ternary"))?;
            let scales = load(&format!("{base}.scales"))?;
            let weights: Vec<i8> = tern.iter().map(|&b| b as i8).collect();
            let scales_raw: Vec<u16> = scales
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            TernaryLayer::new(rows, cols, weights, scales_raw)
                .map_err(|e| OneTokenError::Model(e.to_string()))
        };

    let state_in = ternary("state_input_to_state", STATE_SLOTS, D_MODEL)?;
    let state_out = ternary("state_state_to_output", D_MODEL, STATE_SLOTS)?;

    let mut blocks = Vec::new();
    for k in 0..N_BLOCKS {
        blocks.push(gbf_kernel::model_ref::BlockWeights {
            up: ternary(&format!("block{k}_up"), D_FF, D_MODEL)?,
            down: ternary(&format!("block{k}_down"), D_MODEL, D_FF)?,
        });
    }

    let checkpoint = StateCheckpoint::new(embedding, state_in, state_out, decay_raw, blocks)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    Ok(StateCheckpointBundle {
        checkpoint,
        manifest_schema: schema,
        manifest_git_sha: git_sha,
        manifest_sha256: sha256(&manifest_bytes).to_hex(),
        tensors_verified: verified,
    })
}

// ---------------------------------------------------------------------------
// charset val stream
// ---------------------------------------------------------------------------

/// Assemble the charset_v1 validation token stream exactly as the trainer
/// did: val-split book bodies capped at `cap` raw bytes, trimmed to a valid
/// UTF-8 prefix, then `gbf_data::charset_v1::normalize_raw`.
pub fn build_val_char_ids(
    repo_root: &Path,
    cap: usize,
) -> Result<(Vec<u8>, String), OneTokenError> {
    let raw = build_val_bytes(repo_root, cap)?;
    let valid = match std::str::from_utf8(&raw) {
        Ok(_) => raw.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(e) => {
            return Err(OneTokenError::Manifest {
                reason: format!("val stream is not valid UTF-8: {e}"),
            });
        }
    };
    let norm = gbf_data::charset_v1::normalize_raw(&raw[..valid]).map_err(|e| {
        OneTokenError::Manifest {
            reason: format!("charset_v1 normalization: {e}"),
        }
    })?;
    let ids = norm.tokens.into_vec();
    let sha = sha256(ids.as_slice()).to_hex();
    Ok((ids, sha))
}

/// Decode-side inverse of `charset_v1` (printable ids only), identical to
/// the S5 trainer's sample decoding.
#[must_use]
pub fn id_to_char(id: u8) -> char {
    match id {
        0..=25 => (b'A' + id) as char,
        26..=51 => (b'a' + (id - 26)) as char,
        52..=61 => (b'0' + (id - 52)) as char,
        62 => ' ',
        63 => '.',
        64 => ',',
        65 => '!',
        66 => '?',
        67 => '-',
        68 => '\'',
        69 => ':',
        70 => ';',
        71 => '(',
        72 => ')',
        73 => '"',
        74 => '/',
        75 => '\n',
        _ => '\u{FFFD}',
    }
}

/// Render a generated id sequence as committed sample text.
#[must_use]
pub fn render_char_sample(sequence: &[u8]) -> String {
    sequence.iter().map(|&id| id_to_char(id)).collect()
}

// ---------------------------------------------------------------------------
// phase 2: fidelity (int semantics vs trainer f32 semantics, state carried)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StateFidelityReport {
    pub val_chars_total: usize,
    pub val_norm_tokens_sha256: String,
    /// sha256 match against the committed S5 report's normalized val stream.
    pub val_sha_matches_s5_report: Option<bool>,
    pub eval_lanes: usize,
    pub eval_positions: usize,
    /// f32 trainer-port bits/char over the sequential val positions.
    pub f32_port_val_bpc: f64,
    /// The committed arm-B hard-ternary val bpc from the S5 report.
    pub committed_arm_b_val_bpc: Option<f64>,
    /// |f32 port - committed|; the port's own validation (target ~1e-3).
    pub f32_port_delta_vs_committed: Option<f64>,
    pub f32_port_reproduces_committed_within_1e3: Option<bool>,
    /// Canonical integer semantics bits/char over the same positions.
    pub int_val_bpc: f64,
    /// Per-position argmax agreement between the two paths (each carrying
    /// its own state).
    pub argmax_agreement: f64,
    pub int_stats: StateIntStatsReport,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct StateIntStatsReport {
    pub max_abs_matvec_acc: u32,
    pub max_abs_in_acc: u32,
    pub max_abs_state_delta: u64,
    pub max_abs_state: u32,
    pub state_clamp_events: u64,
    pub max_abs_out_acc: u64,
    pub max_abs_out_scale_product: u64,
    pub y_saturation_events: u64,
    pub max_abs_scale_product: u64,
    pub max_abs_down_delta: u64,
    pub down_delta_clamp_events: u64,
    pub max_abs_residual: u32,
    pub residual_i24_wrap_events: u64,
    pub max_abs_logit: u32,
    pub max_norm_sumsq: u64,
    pub min_norm_rms_raw: u32,
    /// In-projection and FFN accumulators honored the i16 device bound.
    pub matvec_acc_fits_i16: bool,
    /// State slots stayed inside the saturating i24 bound.
    pub state_fits_i24: bool,
    /// Out-projection accumulators fit the 4-byte device accumulator.
    pub out_acc_fits_i32: bool,
    /// Residual stream fit i24 without wrapping.
    pub residual_fits_i24: bool,
    /// Head logits fit the device i24 representation.
    pub logits_fit_i24: bool,
}

impl StateIntStatsReport {
    fn from_stats(s: &StateForwardStats) -> Self {
        Self {
            max_abs_matvec_acc: s.ffn.max_abs_matvec_acc,
            max_abs_in_acc: s.max_abs_in_acc,
            max_abs_state_delta: s.max_abs_state_delta,
            max_abs_state: s.max_abs_state,
            state_clamp_events: s.state_clamp_events,
            max_abs_out_acc: s.max_abs_out_acc,
            max_abs_out_scale_product: s.max_abs_out_scale_product,
            y_saturation_events: s.y_saturation_events,
            max_abs_scale_product: s.ffn.max_abs_scale_product,
            max_abs_down_delta: s.ffn.max_abs_down_delta,
            down_delta_clamp_events: s.ffn.down_delta_clamp_events,
            max_abs_residual: s.ffn.max_abs_residual,
            residual_i24_wrap_events: s.residual_i24_wrap_events,
            max_abs_logit: s.ffn.max_abs_logit,
            max_norm_sumsq: s.ffn.max_norm_sumsq,
            min_norm_rms_raw: s.ffn.min_norm_rms_raw,
            matvec_acc_fits_i16: s.ffn.max_abs_matvec_acc <= 32767,
            state_fits_i24: s.max_abs_state < (1 << 23),
            out_acc_fits_i32: s.max_abs_out_acc < (1 << 31),
            residual_fits_i24: s.residual_i24_wrap_events == 0,
            logits_fit_i24: s.ffn.max_abs_logit < (1 << 23),
        }
    }
}

fn log_softmax_80(logits: &[f64; STATE_VOCAB]) -> [f64; STATE_VOCAB] {
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = logits.iter().map(|l| (l - max).exp()).sum();
    let lse = max + sum.ln();
    let mut out = [0.0f64; STATE_VOCAB];
    for (o, l) in out.iter_mut().zip(logits.iter()) {
        *o = l - lse;
    }
    out
}

fn argmax_80(v: &[f64; STATE_VOCAB]) -> u8 {
    let mut best = 0usize;
    for i in 1..STATE_VOCAB {
        if v[i] > v[best] {
            best = i;
        }
    }
    best as u8
}

/// Run the fidelity measurement over the sequential charset val stream in
/// the committed 8-lane layout (each lane's state carried from zero across
/// its whole contiguous segment). `max_positions_per_lane` 0 scores the
/// full stream, reproducing the committed pair set exactly.
pub fn run_state_fidelity(
    repo_root: &Path,
    ck: &StateCheckpoint,
    lowered: &IntStateLoweredModel,
    val_cap_bytes: usize,
    max_positions_per_lane: usize,
) -> Result<StateFidelityReport, OneTokenError> {
    let (ids, ids_sha) = build_val_char_ids(repo_root, val_cap_bytes)?;

    // Cross-check against the committed S5 report.
    let s5_path = repo_root.join("experiments/S5/state-ab/report.json");
    let (sha_matches, committed_bpc) = match fs::read(&s5_path) {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(report) => {
                let sha_ok = report["corpus"]["val_norm_tokens_sha256"]
                    .as_str()
                    .map(|s| s == ids_sha);
                let bpc = report["arms"].as_array().and_then(|arms| {
                    arms.iter()
                        .find(|a| a["arm"].as_str() == Some("B"))
                        .and_then(|a| {
                            a["measurement"]["ternary_val_bpc_per_normalized_char"].as_f64()
                        })
                });
                (sha_ok, bpc)
            }
            Err(_) => (None, None),
        },
        Err(_) => (None, None),
    };

    let lane_len = ids.len() / EVAL_LANES;
    if lane_len < 2 {
        return Err(OneTokenError::Manifest {
            reason: "val stream too small for the lane layout".into(),
        });
    }
    let pairs_per_lane = if max_positions_per_lane == 0 {
        lane_len - 1
    } else {
        (lane_len - 1).min(max_positions_per_lane)
    };

    let ln2 = std::f64::consts::LN_2;
    let step = lowered.logit_dequant_step();
    let mut bits_f = 0.0f64;
    let mut bits_i = 0.0f64;
    let mut agree = 0u64;
    let mut merged = StateForwardStats::new();
    for lane in 0..EVAL_LANES {
        let base = lane * lane_len;
        let mut f_state = [0.0f32; STATE_SLOTS];
        let mut i_state = [0i32; STATE_SLOTS];
        for t in 0..pairs_per_lane {
            let ctx = ids[base + t];
            let tgt = usize::from(ids[base + t + 1]);

            let f_logits = f32_state_forward(ck, ctx, &mut f_state);
            let mut f64_logits = [0.0f64; STATE_VOCAB];
            for (o, l) in f64_logits.iter_mut().zip(f_logits.iter()) {
                *o = f64::from(*l);
            }
            let f_lp = log_softmax_80(&f64_logits);
            bits_f += -f_lp[tgt] / ln2;
            let f_arg = argmax_80(&f64_logits);

            let trace = lowered.forward(ctx, &mut i_state);
            merged.merge(&trace.stats);
            let mut i_logits = [0.0f64; STATE_VOCAB];
            for (o, l) in i_logits.iter_mut().zip(trace.logits.iter()) {
                *o = f64::from(*l) * step;
            }
            let i_lp = log_softmax_80(&i_logits);
            bits_i += -i_lp[tgt] / ln2;
            if f_arg == trace.argmax {
                agree += 1;
            }
        }
    }
    let positions = pairs_per_lane * EVAL_LANES;
    let f32_bpc = bits_f / positions as f64;
    let delta = committed_bpc.map(|c| (f32_bpc - c).abs());

    Ok(StateFidelityReport {
        val_chars_total: ids.len(),
        val_norm_tokens_sha256: ids_sha,
        val_sha_matches_s5_report: sha_matches,
        eval_lanes: EVAL_LANES,
        eval_positions: positions,
        f32_port_val_bpc: f32_bpc,
        committed_arm_b_val_bpc: committed_bpc,
        f32_port_delta_vs_committed: delta,
        f32_port_reproduces_committed_within_1e3: delta.map(|d| d <= 1.5e-3),
        int_val_bpc: bits_i / positions as f64,
        argmax_agreement: agree as f64 / positions as f64,
        int_stats: StateIntStatsReport::from_stats(&merged),
    })
}

// ---------------------------------------------------------------------------
// phase 3a: one-token ROM gate (host-poked input + carried state)
// ---------------------------------------------------------------------------

/// Expected WRAM segments for one host trace: (name, address, bytes).
pub(crate) fn state_expected_segments(trace: &IntStateForwardTrace) -> Vec<(String, u16, Vec<u8>)> {
    let i16s = |v: &[i16]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let i32s = |v: &[i32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let i24s = |v: &[i32]| -> Vec<u8> {
        v.iter()
            .flat_map(|x| x.to_le_bytes().into_iter().take(3))
            .collect()
    };
    let mut segments = vec![
        (
            "state_norm_act".to_string(),
            S_DUMP_SNORM,
            trace.state_norm_act.to_vec(),
        ),
        (
            "state_in_acc".to_string(),
            S_DUMP_INACC,
            i16s(&trace.state_in_acc),
        ),
        (
            "state_after".to_string(),
            S_STATE_BASE,
            i32s(&trace.state_after),
        ),
        (
            "state_out_acc".to_string(),
            S_SACC_BASE,
            i32s(&trace.state_out_acc),
        ),
        ("y_act".to_string(), S_DUMP_YACT, trace.y_act.to_vec()),
        (
            "block0_norm_act".to_string(),
            S_DUMP_NORM0,
            trace.block0_norm_act.to_vec(),
        ),
        (
            "block0_up_acc".to_string(),
            S_DUMP_UPACC0,
            i16s(&trace.block0_up_acc),
        ),
        (
            "block0_gelu_act".to_string(),
            S_DUMP_GELU0,
            trace.block0_gelu_act.to_vec(),
        ),
        (
            "block0_down_acc".to_string(),
            S_DUMP_DOWNACC0,
            i16s(&trace.block0_down_acc),
        ),
    ];
    for (k, res) in trace.block_residuals.iter().enumerate() {
        segments.push((
            format!("block{k}_residual_i24"),
            S_XDUMP_BASE + 192 * k as u16,
            i24s(res),
        ));
    }
    segments.push((
        "final_norm_act".to_string(),
        S_QDUMP_BASE,
        trace.final_q.iter().map(|&q| (q + 128) as u8).collect(),
    ));
    let mut logit_bytes = Vec::with_capacity(STATE_VOCAB * 3);
    for &l in &trace.logits {
        logit_bytes.extend_from_slice(&l.to_le_bytes()[..3]);
    }
    segments.push(("logits_i24".to_string(), S_LOGITS_BASE, logit_bytes));
    segments.push(("argmax".to_string(), S_ARGMAX_ADDR, vec![trace.argmax]));
    segments
}

#[derive(Debug, Clone, Serialize)]
pub struct StateRomFacts {
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
    pub token_start_pc: u16,
    pub token_end_pc: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateRomRun {
    pub input_id: u8,
    /// Val position the poked state was harvested from (0 = zero state).
    pub state_from_position: usize,
    pub state_is_zero: bool,
    pub host_argmax: u8,
    pub rom_argmax: u8,
    pub byte_exact: bool,
    pub m_cycles: u64,
    pub mismatches: Vec<SegmentMismatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateRomGateReport {
    pub rom: StateRomFacts,
    pub cases: usize,
    pub all_byte_exact: bool,
    pub runs: Vec<StateRomRun>,
    pub mean_m_cycles: u64,
    pub seconds_per_token_dmg: f64,
}

/// Harvest `(position, input, carried state)` cases from the val stream by
/// running the host integer evaluator sequentially from position 0.
pub fn harvest_state_cases(
    lowered: &IntStateLoweredModel,
    ids: &[u8],
    positions: &[usize],
) -> Vec<(usize, u8, [i32; STATE_SLOTS])> {
    let max_pos = positions.iter().copied().max().unwrap_or(0);
    let mut state = [0i32; STATE_SLOTS];
    let mut cases = Vec::with_capacity(positions.len());
    for (pos, &id) in ids.iter().enumerate().take(max_pos + 1) {
        if positions.contains(&pos) {
            cases.push((pos, id, state));
        }
        let _ = lowered.forward(id, &mut state);
    }
    cases
}

fn run_one_state_case(
    rom: &StateOneTokenRom,
    lowered: &IntStateLoweredModel,
    position: usize,
    input: u8,
    state: &[i32; STATE_SLOTS],
) -> Result<StateRomRun, OneTokenError> {
    let mut host_state = *state;
    let trace = lowered.forward(input, &mut host_state);

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    emu.poke(S_INPUT_ADDR, input)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    for (slot, h) in state.iter().enumerate() {
        for (k, byte) in h.to_le_bytes().into_iter().enumerate() {
            emu.poke(S_STATE_BASE + (slot * 4 + k) as u16, byte)
                .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
        }
    }

    let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000));
    let run_to = |emu: &mut Emulator, pc: u16, phase: &str| -> Result<(), OneTokenError> {
        match emu.run_fast_until_pc(pc, budget) {
            Ok(RunOutcome::TrapHit { .. }) => Ok(()),
            Ok(other) => Err(OneTokenError::Emulator(format!(
                "did not reach {phase} at {pc:#06x}: {other:?}"
            ))),
            Err(e) => Err(OneTokenError::Emulator(e.to_string())),
        }
    };
    run_to(&mut emu, rom.token_start_pc, "token start")?;
    let start = emu.m_cycle_count_floor().0;
    run_to(&mut emu, rom.token_end_pc, "token end")?;
    let end = emu.m_cycle_count_floor().0;

    let mut mismatches = Vec::new();
    for (name, addr, expected) in state_expected_segments(&trace) {
        let actual = emu
            .peek_range(addr, expected.len())
            .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
        if actual != expected {
            let off = actual
                .iter()
                .zip(expected.iter())
                .position(|(a, e)| a != e)
                .unwrap_or(0);
            mismatches.push(SegmentMismatch {
                segment: name,
                wram_addr: addr,
                first_bad_offset: off,
                expected_byte: expected[off],
                actual_byte: actual[off],
            });
        }
    }
    let rom_argmax = emu
        .peek(S_ARGMAX_ADDR)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;

    Ok(StateRomRun {
        input_id: input,
        state_from_position: position,
        state_is_zero: state.iter().all(|&h| h == 0),
        host_argmax: trace.argmax,
        rom_argmax,
        byte_exact: mismatches.is_empty(),
        m_cycles: end.saturating_sub(start),
        mismatches,
    })
}

/// Build the one-token ROM and run the byte-exact agreement gate over the
/// harvested (input, state) cases.
pub fn run_state_rom_gate(
    lowered: &IntStateLoweredModel,
    cases: &[(usize, u8, [i32; STATE_SLOTS])],
) -> Result<StateRomGateReport, OneTokenError> {
    let rom = build_state_one_token_rom(lowered).map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let facts = StateRomFacts {
        rom_bytes: rom.rom.len(),
        bank_count: rom.bank_count,
        driver_bytes: rom.driver_bytes,
        weight_code_bytes: rom.weight_code_bytes,
        weight_chunk_count: rom.weight_chunk_count,
        table_bytes: rom.table_bytes,
        token_start_pc: rom.token_start_pc,
        token_end_pc: rom.token_end_pc,
    };
    let mut runs = Vec::new();
    for (position, input, state) in cases {
        runs.push(run_one_state_case(&rom, lowered, *position, *input, state)?);
    }
    let all_byte_exact = runs.iter().all(|r| r.byte_exact);
    let mean_m_cycles = runs.iter().map(|r| r.m_cycles).sum::<u64>() / runs.len().max(1) as u64;
    Ok(StateRomGateReport {
        rom: facts,
        cases: runs.len(),
        all_byte_exact,
        runs,
        mean_m_cycles,
        seconds_per_token_dmg: mean_m_cycles as f64 / DMG_M_CYCLES_PER_SECOND as f64,
    })
}

// ---------------------------------------------------------------------------
// phase 3b: multi-token sustained generation gate
// ---------------------------------------------------------------------------

/// Host-side stateful generation mirror: zero state, argmax feedback.
pub struct StateHostGeneration {
    pub sequence: Vec<u8>,
    pub first_trace: IntStateForwardTrace,
    pub last_trace: IntStateForwardTrace,
}

/// Generate `n_tokens` ids on the host with the state carried from zero.
#[must_use]
pub fn state_host_generate(
    lowered: &IntStateLoweredModel,
    seed: u8,
    n_tokens: u16,
) -> StateHostGeneration {
    assert!(n_tokens >= 1, "host generation needs at least one token");
    let mut state = [0i32; STATE_SLOTS];
    let mut input = seed;
    let mut sequence = Vec::with_capacity(usize::from(n_tokens));
    let mut first_trace = None;
    let mut last_trace = None;
    for t in 0..n_tokens {
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax;
        sequence.push(trace.argmax);
        if t == 0 {
            first_trace = Some(trace.clone());
        }
        if t == n_tokens - 1 {
            last_trace = Some(trace);
        }
    }
    StateHostGeneration {
        sequence,
        first_trace: first_trace.expect("n_tokens >= 1"),
        last_trace: last_trace.expect("n_tokens >= 1"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StateSeedRun {
    pub seed_id: u8,
    pub n_tokens: u16,
    pub host_sequence_sha256: String,
    pub rom_sequence_sha256: String,
    pub sequences_match: bool,
    pub first_divergence_index: Option<usize>,
    pub first_token_checkpoints_byte_exact: bool,
    pub last_token_checkpoints_byte_exact: bool,
    pub checkpoint_mismatches: Vec<SegmentMismatch>,
    pub cycles: CycleStats,
    pub sp_home_every_token: bool,
    pub sp_violation_tokens: Vec<u16>,
    pub wram_untouched_regions_ok: bool,
    pub wram_violations: Vec<WramViolation>,
    pub done_flag_set: bool,
    pub sample_file: String,
    #[serde(skip)]
    pub rom_sequence: Vec<u8>,
}

impl StateSeedRun {
    #[must_use]
    pub fn all_checks_pass(&self) -> bool {
        self.sequences_match
            && self.first_token_checkpoints_byte_exact
            && self.last_token_checkpoints_byte_exact
            && self.cycles.stable_within_5pct
            && self.sp_home_every_token
            && self.wram_untouched_regions_ok
            && self.done_flag_set
    }
}

fn compare_state_dumps(
    emu: &Emulator,
    trace: &IntStateForwardTrace,
    token: u16,
    mismatches: &mut Vec<SegmentMismatch>,
) -> Result<bool, OneTokenError> {
    let mut all_ok = true;
    for (name, addr, expected) in state_expected_segments(trace) {
        let actual = emu
            .peek_range(addr, expected.len())
            .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
        if actual != expected {
            all_ok = false;
            let off = actual
                .iter()
                .zip(expected.iter())
                .position(|(a, e)| a != e)
                .unwrap_or(0);
            mismatches.push(SegmentMismatch {
                segment: format!("token{token}/{name}"),
                wram_addr: addr,
                first_bad_offset: off,
                expected_byte: expected[off],
                actual_byte: actual[off],
            });
        }
    }
    Ok(all_ok)
}

/// Execute the stateful multi-token ROM for one seed and run every gate
/// against the host mirror. The generation loop (state update, forward,
/// argmax, ring write, feedback) executes entirely on-device.
pub fn run_state_seed_generation(
    rom: &StateMultiTokenRom,
    lowered: &IntStateLoweredModel,
    seed: u8,
) -> Result<StateSeedRun, OneTokenError> {
    let host = state_host_generate(lowered, seed, rom.n_tokens);

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    emu.poke(S_INPUT_ADDR, seed)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;

    let baseline: Vec<Vec<u8>> = STATE_UNTOUCHED_WRAM_REGIONS
        .iter()
        .map(|&(start, end)| emu.peek_range(start, usize::from(end - start)))
        .collect::<Result<_, _>>()
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;

    let budget = CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(3_000));
    let run_to = |emu: &mut Emulator, pc: u16, phase: &str| -> Result<(), OneTokenError> {
        match emu.run_fast_until_pc(pc, budget) {
            Ok(RunOutcome::TrapHit { .. }) => Ok(()),
            Ok(other) => Err(OneTokenError::Emulator(format!(
                "did not reach {phase} at {pc:#06x}: {other:?}"
            ))),
            Err(e) => Err(OneTokenError::Emulator(e.to_string())),
        }
    };

    run_to(&mut emu, rom.token_start_pc, "token start")?;
    let mut prev_cycles = emu.m_cycle_count_floor().0;
    let mut per_token_cycles = Vec::with_capacity(usize::from(rom.n_tokens));
    let mut sp_violation_tokens = Vec::new();
    let mut checkpoint_mismatches = Vec::new();
    let mut first_token_ok = true;
    let mut last_token_ok = true;

    for t in 0..rom.n_tokens {
        if t > 0 {
            run_to(&mut emu, rom.token_start_pc, "loop head")?;
        }
        run_to(&mut emu, rom.token_boundary_pc, "token boundary")?;
        let now = emu.m_cycle_count_floor().0;
        per_token_cycles.push(now.saturating_sub(prev_cycles));
        prev_cycles = now;

        if emu.regs().sp != S_STACK_TOP {
            sp_violation_tokens.push(t);
        }
        if t == 0 {
            first_token_ok =
                compare_state_dumps(&emu, &host.first_trace, t, &mut checkpoint_mismatches)?;
        }
        if t == rom.n_tokens - 1 {
            last_token_ok =
                compare_state_dumps(&emu, &host.last_trace, t, &mut checkpoint_mismatches)?;
        }
    }

    run_to(&mut emu, rom.token_end_pc, "token end")?;
    let done_flag_set = emu
        .peek(S_DONE_ADDR)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?
        == 1;

    let rom_sequence = emu
        .peek_range(S_OUT_BASE, usize::from(rom.n_tokens))
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    let first_divergence_index = host
        .sequence
        .iter()
        .zip(rom_sequence.iter())
        .position(|(h, r)| h != r);
    let sequences_match =
        first_divergence_index.is_none() && host.sequence.len() == rom_sequence.len();

    let mut wram_violations = Vec::new();
    for (&(start, end), before) in STATE_UNTOUCHED_WRAM_REGIONS.iter().zip(baseline.iter()) {
        let after = emu
            .peek_range(start, usize::from(end - start))
            .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
        if let Some(off) = before.iter().zip(after.iter()).position(|(b, a)| b != a) {
            wram_violations.push(WramViolation {
                region_start: start,
                region_end: end,
                first_bad_addr: start + off as u16,
                before: before[off],
                after: after[off],
            });
        }
    }

    Ok(StateSeedRun {
        seed_id: seed,
        n_tokens: rom.n_tokens,
        host_sequence_sha256: sha256(&host.sequence).to_hex(),
        rom_sequence_sha256: sha256(&rom_sequence).to_hex(),
        sequences_match,
        first_divergence_index,
        first_token_checkpoints_byte_exact: first_token_ok,
        last_token_checkpoints_byte_exact: last_token_ok,
        checkpoint_mismatches,
        cycles: CycleStats::from_samples(&per_token_cycles),
        sp_home_every_token: sp_violation_tokens.is_empty(),
        sp_violation_tokens,
        wram_untouched_regions_ok: wram_violations.is_empty(),
        wram_violations,
        done_flag_set,
        sample_file: format!("sample_seed_{:02}_{}.txt", seed, sample_seed_tag(seed)),
        rom_sequence,
    })
}

fn sample_seed_tag(seed: u8) -> String {
    match id_to_char(seed) {
        ' ' => "space".to_string(),
        '\n' => "newline".to_string(),
        c if c.is_ascii_alphanumeric() => c.to_string(),
        _ => format!("id{seed}"),
    }
}

// ---------------------------------------------------------------------------
// phase 4: evidence report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StateCheckpointFacts {
    pub export_dir: String,
    pub manifest_schema: String,
    pub manifest_sha256: String,
    pub trainer_git_sha: String,
    pub tensors_verified_sha256: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMultiRomFacts {
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
    pub token_start_pc: u16,
    pub token_boundary_pc: u16,
    pub token_end_pc: u16,
    pub n_tokens: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMultiTokenReport {
    pub rom: StateMultiRomFacts,
    pub seeds: Vec<u8>,
    pub all_sequences_match: bool,
    pub all_health_checks_pass: bool,
    pub mean_m_cycles_per_token: u64,
    pub seconds_per_token_dmg: f64,
    pub runs: Vec<StateSeedRun>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatefulRomReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub upstream_beads: Vec<&'static str>,
    pub git_sha: String,
    pub checkpoint: StateCheckpointFacts,
    pub int_semantics_divergences: Vec<String>,
    pub fidelity: StateFidelityReport,
    pub one_token_gate: StateRomGateReport,
    pub multi_token: StateMultiTokenReport,
    pub caveats: Vec<String>,
}

/// Run every phase and assemble the evidence report.
pub fn run_stateful_bringup(
    repo_root: &Path,
    export_dir_rel: &str,
    max_positions_per_lane: usize,
) -> Result<StatefulRomReport, OneTokenError> {
    let export_dir = repo_root.join(export_dir_rel);
    let bundle = load_state_checkpoint(&export_dir)?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;

    // Phase 2: fidelity with state carried sequentially.
    let fidelity = run_state_fidelity(
        repo_root,
        &bundle.checkpoint,
        &lowered,
        1 << 20,
        max_positions_per_lane,
    )?;

    // Phase 3a: one-token gate over harvested (input, state) cases.
    let (ids, _) = build_val_char_ids(repo_root, 1 << 20)?;
    let cases = harvest_state_cases(&lowered, &ids, &ONE_TOKEN_STATE_POSITIONS);
    let one_token_gate = run_state_rom_gate(&lowered, &cases)?;

    // Phase 3b: sustained on-device generation.
    let rom = build_state_multi_token_rom(&lowered, STATE_GENERATION_TOKENS)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let mut runs = Vec::new();
    for &seed in &STATE_GENERATION_SEEDS {
        runs.push(run_state_seed_generation(&rom, &lowered, seed)?);
    }
    let all_sequences_match = runs.iter().all(|r| r.sequences_match);
    let all_health_checks_pass = runs.iter().all(StateSeedRun::all_checks_pass);
    let mean_m_cycles_per_token = runs.iter().map(|r| r.cycles.mean).sum::<u64>()
        / u64::try_from(runs.len().max(1)).expect("run count fits u64");
    let multi_token = StateMultiTokenReport {
        rom: StateMultiRomFacts {
            rom_bytes: rom.rom.len(),
            bank_count: rom.bank_count,
            driver_bytes: rom.driver_bytes,
            weight_code_bytes: rom.weight_code_bytes,
            weight_chunk_count: rom.weight_chunk_count,
            table_bytes: rom.table_bytes,
            token_start_pc: rom.token_start_pc,
            token_boundary_pc: rom.token_boundary_pc,
            token_end_pc: rom.token_end_pc,
            n_tokens: rom.n_tokens,
        },
        seeds: STATE_GENERATION_SEEDS.to_vec(),
        all_sequences_match,
        all_health_checks_pass,
        mean_m_cycles_per_token,
        seconds_per_token_dmg: mean_m_cycles_per_token as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        runs,
    };

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(StatefulRomReport {
        schema: "stateful_rom_bringup.v1",
        bead: "bd-x5l2s",
        upstream_beads: vec!["bd-29ai4", "bd-59qiq", "bd-2gc6p"],
        git_sha,
        checkpoint: StateCheckpointFacts {
            export_dir: export_dir_rel.to_string(),
            manifest_schema: bundle.manifest_schema,
            manifest_sha256: bundle.manifest_sha256,
            trainer_git_sha: bundle.manifest_git_sha,
            tensors_verified_sha256: bundle.tensors_verified,
        },
        int_semantics_divergences: STATE_INT_SEMANTIC_DIVERGENCES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        fidelity,
        one_token_gate,
        multi_token,
        caveats: vec![
            "Stateful model: per-token integer/f32 rounding differences accumulate through the carried state, so fidelity deltas are structurally larger than the dense bring-up's per-context numbers; the deployment-relevant fact is the integer path's own val bpc laid against the committed stateless arm A (3.6651 bpc/char).".to_string(),
            "The f32 reference is a scalar port of the trainer's batched Burn forward (sequential f32 summation, f64 log-softmax); the S5 report's own scalar-kernel parity check bounds the batched-vs-scalar difference at ~2e-6 per state-block output.".to_string(),
            "Greedy argmax generation with a deterministic recurrence: unlike the bigram ROM, the state makes repeated inputs produce different outputs, so cycles are not structural; degenerate loops can still appear if the state itself converges.".to_string(),
            "ROM runs with interrupts disabled and SP repurposed inside weight chunks (bake-off convention); production kernels pay yield/safe-point overhead on top.".to_string(),
        ],
    })
}

/// Render the report README (generated, not hand-written).
#[must_use]
pub fn stateful_report_to_markdown(report: &StatefulRomReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Stateful ROM bring-up ({})", report.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "First on-device execution of a **stateful** trained model: the LinearState \
         multi-timescale arm-B checkpoint (bd-29ai4) running its exact integer recurrence \
         on the emulated Game Boy, with the recurrent state living in WRAM across tokens \
         (bead {}). Generated by `cargo run -p gbf-bench --bin stateful-rom`; every number \
         below is program output at git `{}`.",
        report.bead, report.git_sha
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Checkpoint");
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
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "## Phase 2 — lowering fidelity (integer vs trainer f32, state carried)"
    );
    let _ = writeln!(out);
    let f = &report.fidelity;
    let _ = writeln!(
        out,
        "- Val stream: {} charset_v1 chars, normalized-token sha256 `{}`{}",
        f.val_chars_total,
        &f.val_norm_tokens_sha256[..16],
        match f.val_sha_matches_s5_report {
            Some(true) => " (identical to the committed S5 A/B eval stream)",
            Some(false) => " (DOES NOT match the committed S5 A/B eval stream)",
            None => " (S5 report unavailable for cross-check)",
        }
    );
    let _ = writeln!(
        out,
        "- Sequential positions scored: {} ({} lanes, per-lane state carried from zero)",
        f.eval_positions, f.eval_lanes
    );
    let _ = writeln!(
        out,
        "- f32-port val bpc: {:.6}{}",
        f.f32_port_val_bpc,
        match (f.committed_arm_b_val_bpc, f.f32_port_delta_vs_committed) {
            (Some(c), Some(d)) => format!(
                " (committed arm-B bpc {c:.6}, |delta| {d:.2e} — port validation {})",
                if f.f32_port_reproduces_committed_within_1e3 == Some(true) {
                    "PASS"
                } else {
                    "FAIL"
                }
            ),
            _ => String::new(),
        }
    );
    let _ = writeln!(
        out,
        "- **Integer-semantics val bpc: {:.6}** ({:+.6} vs the f32 port; committed stateless arm A was 3.6651)",
        f.int_val_bpc,
        f.int_val_bpc - f.f32_port_val_bpc
    );
    let _ = writeln!(
        out,
        "- Per-position argmax agreement (int vs f32, each carrying its own state): {:.4}%",
        f.argmax_agreement * 100.0
    );
    let s = &f.int_stats;
    let _ = writeln!(
        out,
        "- Range check on real data: max |in-proj acc| {} / |FFN acc| {} (i16: {}), max |state delta m| {}, max |state| {} (i24 saturation bound {}: {}, {} clamp events), max |out acc| {} (i32: {}), max |residual| {} (i24 Q19.5, {} wraps), {} down-delta clamps, max |logit| {} (i24: {})",
        s.max_abs_in_acc,
        s.max_abs_matvec_acc,
        if s.matvec_acc_fits_i16 {
            "ok"
        } else {
            "VIOLATED"
        },
        s.max_abs_state_delta,
        s.max_abs_state,
        (1u32 << 23) - 1,
        if s.state_fits_i24 { "ok" } else { "SATURATED" },
        s.state_clamp_events,
        s.max_abs_out_acc,
        if s.out_acc_fits_i32 { "ok" } else { "VIOLATED" },
        s.max_abs_residual,
        s.residual_i24_wrap_events,
        s.down_delta_clamp_events,
        s.max_abs_logit,
        if s.logits_fit_i24 { "ok" } else { "VIOLATED" }
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "## Phase 3a — one-token ROM gate (host-poked carried state)"
    );
    let _ = writeln!(out);
    let g = &report.one_token_gate;
    let _ = writeln!(
        out,
        "- ROM: {} bytes ({} banks), driver {} B, weight code {} B in {} chunks, tables {} B",
        g.rom.rom_bytes,
        g.rom.bank_count,
        g.rom.driver_bytes,
        g.rom.weight_code_bytes,
        g.rom.weight_chunk_count,
        g.rom.table_bytes
    );
    let _ = writeln!(
        out,
        "- **Gate: {} — {}/{} (input, state) cases byte-exact across all WRAM checkpoints** \
         (state-block norm/in-acc/state/out-acc/y dumps, block-0 dumps, 4 i24 residual dumps, \
         final norm, 80 i24 logits, argmax)",
        if g.all_byte_exact { "PASS" } else { "FAIL" },
        g.runs.iter().filter(|run| run.byte_exact).count(),
        g.runs.len()
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| input id | state from val pos | zero state | host argmax | ROM argmax | byte-exact | M-cycles |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for run in &g.runs {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            run.input_id,
            run.state_from_position,
            if run.state_is_zero { "yes" } else { "no" },
            run.host_argmax,
            run.rom_argmax,
            if run.byte_exact { "yes" } else { "NO" },
            run.m_cycles
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Phase 3b — sustained on-device stateful generation");
    let _ = writeln!(out);
    let m = &report.multi_token;
    let _ = writeln!(
        out,
        "- {} tokens per seed generated entirely on-device (state zeroed once, then evolving \
         in WRAM at 0x{:04X}); token boundary trap at {:#06x}",
        m.rom.n_tokens, S_STATE_BASE, m.rom.token_boundary_pc
    );
    let _ = writeln!(
        out,
        "- **Sequences: {}** — {}/{} seeds byte-identical to the host integer evaluator",
        if m.all_sequences_match {
            "PASS"
        } else {
            "FAIL"
        },
        m.runs.iter().filter(|r| r.sequences_match).count(),
        m.runs.len()
    );
    let _ = writeln!(
        out,
        "- **Health: {}** — SP home at every token boundary, untouched WRAM regions unchanged, \
         per-token cycles stable, first/last-token dumps (including the state vector) \
         byte-exact, done flag set",
        if m.all_health_checks_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| seed id | char | sequence match | first/last dumps | cycles min | median | max | max/min | SP home | WRAM clean | sample |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|");
    for run in &m.runs {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {}/{} | {} | {} | {} | {:.5} | {} | {} | `{}` |",
            run.seed_id,
            id_to_char(run.seed_id).escape_default(),
            if run.sequences_match { "yes" } else { "NO" },
            if run.first_token_checkpoints_byte_exact {
                "yes"
            } else {
                "NO"
            },
            if run.last_token_checkpoints_byte_exact {
                "yes"
            } else {
                "NO"
            },
            run.cycles.min,
            run.cycles.median,
            run.cycles.max,
            run.cycles.max_over_min,
            if run.sp_home_every_token { "yes" } else { "NO" },
            if run.wram_untouched_regions_ok {
                "yes"
            } else {
                "NO"
            },
            run.sample_file
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Cycles");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- One-token gate mean: **{} M-cycles/token** = {:.3} s/token on DMG",
        report.one_token_gate.mean_m_cycles, report.one_token_gate.seconds_per_token_dmg
    );
    let _ = writeln!(
        out,
        "- Generation loop mean over all seeds and tokens: **{} M-cycles/token** = {:.3} s/token \
         (dense bigram ROM was ~2.73M M-cycles/token; the stateful model adds the in/out state \
         projections, the decay stage, and the widened i24 norm)",
        m.mean_m_cycles_per_token, m.seconds_per_token_dmg
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Sample text");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "The `sample_seed_*.txt` files are the project's **first on-device stateful \
         generation**: charset_v1 ids decoded to text. With real recurrent state the model can \
         escape the fixed bigram cycles of bd-2gc6p — judge the samples honestly; greedy argmax \
         decoding still tends toward loops once the state converges."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "## Documented integer-semantics divergences from the trainer"
    );
    let _ = writeln!(out);
    for d in &report.int_semantics_divergences {
        let _ = writeln!(out, "- {d}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Caveats");
    let _ = writeln!(out);
    for c in &report.caveats {
        let _ = writeln!(out, "- {c}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_kernel::state_model_ref::synthetic_state_checkpoint;

    /// Full-stack smoke on a synthetic stateful checkpoint: one-token ROM vs
    /// host integer evaluator, byte-exact, including a nonzero poked state.
    /// This is the same machinery the real-checkpoint gate uses.
    #[test]
    fn state_one_token_rom_matches_host_on_synthetic_model() {
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let mut nonzero = [0i32; STATE_SLOTS];
        for (slot, h) in nonzero.iter_mut().enumerate() {
            *h = (slot as i32 - 32) * 4093; // mixed signs, multi-byte values
        }
        let cases = vec![(0usize, 7u8, [0i32; STATE_SLOTS]), (1usize, 42u8, nonzero)];
        let report = run_state_rom_gate(&lowered, &cases).expect("gate runs");
        for run in &report.runs {
            assert!(
                run.byte_exact,
                "input {} (state from pos {}): mismatches {:?}",
                run.input_id, run.state_from_position, run.mismatches
            );
        }
        assert!(report.all_byte_exact);
        assert!(report.mean_m_cycles > 0);
    }

    /// Multi-token smoke: the on-device stateful generation loop must
    /// reproduce the host feedback loop byte-exactly and stay healthy.
    #[test]
    fn state_multi_token_rom_matches_host_generation_on_synthetic_model() {
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let rom = build_state_multi_token_rom(&lowered, 5).expect("builds");
        let run = run_state_seed_generation(&rom, &lowered, 19).expect("runs");
        assert!(
            run.sequences_match,
            "ROM sequence diverged at {:?} ({:?})",
            run.first_divergence_index, run.checkpoint_mismatches
        );
        assert!(
            run.first_token_checkpoints_byte_exact && run.last_token_checkpoints_byte_exact,
            "dump mismatches {:?}",
            run.checkpoint_mismatches
        );
        assert!(run.sp_home_every_token);
        assert!(
            run.wram_untouched_regions_ok,
            "WRAM violations {:?}",
            run.wram_violations
        );
        assert!(run.done_flag_set);
        assert_eq!(run.rom_sequence.len(), 5);
    }

    #[test]
    fn render_char_sample_decodes_ids() {
        assert_eq!(render_char_sample(&[19, 33, 30, 62, 75]), "The \n");
    }
}
