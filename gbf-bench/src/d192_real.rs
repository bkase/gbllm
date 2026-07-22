//! REAL d192 checkpoint bring-up (bd-pp43d, phase 2): the committed S8
//! distilled student export (`experiments/S8/sweep/checkpoint-export-
//! CUSTOM_distill`, ternary d192/ff384/6blk/slots192 charset-80, bd-3771m)
//! through the exact production pipeline the synthetic d192-readiness gate
//! (`crate::d192`) pre-cleared: production loader (manifest topology +
//! per-tensor sha256), integer lowering (width plan; `DownEpilogueOverflow`
//! fires loudly on hostile scale/acc products), banked ROM builders
//! (`DriverOverflowsBank0` fires loudly past the bank-0 window), byte-exact
//! emulator gates, full-stream fidelity vs the trainer's hard-ternary f32
//! semantics, the interactive shell's scripted joypad session, and
//! qualitative sampled text with ROM-verified prefixes.
//!
//! Every phase is gated on the previous one; failures are recorded honestly
//! in the report (`d192_real_bringup.v1`) and later phases are skipped, so
//! the evidence under `docs/experiments/d192-real/` always states exactly
//! how far the real checkpoint got. Produced by the `d192-real` bin — never
//! hand-written.

use std::fs;
use std::path::Path;

use gbf_emu::Framebuffer;
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_shell::{
    build_state_shell_rom, build_state_shell_rom_lowered, synthetic_font_tiles,
};
use gbf_kernel::asm_impl_state::{
    STATE_DRIVER_BANK_CAPACITY, StateWramLayout, WeightLowering, build_state_multi_token_rom,
    build_state_multi_token_rom_lowered, build_state_multi_token_sampling_rom_lowered,
    build_state_one_token_rom,
};
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_trace};
use gbf_kernel::model_ref::TernaryLayer;
use gbf_kernel::state_model_ref::{
    IntStateLoweredModel, STATE_INT_SEMANTIC_DIVERGENCES, StateCheckpoint, StateForwardStats,
    synthetic_state_checkpoint_with,
};
use serde::Serialize;

use crate::d192::{
    D192_SYNTHETIC_SEED, D192LayoutFacts, D192MultiTokenReport, D192WidthFacts, TopologyFacts,
};
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};
use crate::sampling::SamplerSettingFacts;
use crate::shell::{char_to_id, run_shell_bringup, run_shell_session, shell_font_tiles};
use crate::stateful::{
    STATE_GENERATION_SEEDS, StateCheckpointFacts, StateIntStatsReport, StateRomGateReport,
    argmax_v, build_val_char_ids, harvest_state_cases, load_state_checkpoint, log_softmax_v,
    render_char_sample, run_state_rom_gate, run_state_rom_gate_lowered, run_state_seed_generation,
};

/// The committed real distilled-student export (bd-3771m).
pub const D192_REAL_EXPORT_DIR: &str = "experiments/S8/sweep/checkpoint-export-CUSTOM_distill";

/// The committed sweep-arm record carrying the trainer's own measurement.
pub const D192_REAL_ARM_JSON: &str = "experiments/S8/sweep/arm_CUSTOM_distill.json";

/// The committed synthetic readiness evidence (cycle prediction source).
pub const D192_READINESS_REPORT: &str = "docs/experiments/d192-readiness/report.json";

/// One-token gate: val-stream positions whose carried state (and input)
/// become the poked cases; position 0 exercises the zero state.
pub const D192_REAL_ONE_TOKEN_POSITIONS: [usize; 5] = [0, 1, 127, 1023, 16383];

/// |f32 port - committed| acceptance bound when scoring the committed pair
/// set exactly (the port's own validation).
pub const F32_PORT_TOLERANCE: f64 = 1.0e-3;

// ---------------------------------------------------------------------------
// options
// ---------------------------------------------------------------------------

/// Gate sizes. `full()` is the evidence configuration; `quick()` exists only
/// for development smoke runs and must never produce committed evidence.
#[derive(Debug, Clone, Serialize)]
pub struct D192RealOptions {
    /// On-device tokens per seed for the multi-token gate (>= 128 for
    /// evidence).
    pub multi_token_tokens: u16,
    /// Multi-token gate seeds (>= 4 for evidence).
    pub multi_token_seeds: Vec<u8>,
    /// Fidelity positions per lane; 0 scores the full committed pair set.
    pub fidelity_positions_per_lane: usize,
    /// Host-side sample length in chars.
    pub sample_chars: usize,
    /// Shell generation cap per session (ROM-verified sample prefix length
    /// is bounded by this and by the 200-cell transcript).
    pub shell_gen_tokens: u8,
    /// Marks quick development runs so the report self-identifies.
    pub quick_mode: bool,
}

impl D192RealOptions {
    #[must_use]
    pub fn full() -> Self {
        Self {
            multi_token_tokens: 128,
            multi_token_seeds: STATE_GENERATION_SEEDS.to_vec(),
            fidelity_positions_per_lane: 0,
            sample_chars: 512,
            shell_gen_tokens: 200,
            quick_mode: false,
        }
    }

    #[must_use]
    pub fn quick() -> Self {
        Self {
            multi_token_tokens: 4,
            multi_token_seeds: vec![19, 62],
            fidelity_positions_per_lane: 64,
            sample_chars: 48,
            shell_gen_tokens: 8,
            quick_mode: true,
        }
    }
}

// ---------------------------------------------------------------------------
// committed provenance (trainer's own numbers, read from the committed json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CommittedProvenance {
    pub arm_json: String,
    pub arm: String,
    pub ternary_val_bpc_per_normalized_char: f64,
    pub ternary_val_bits_per_raw_byte: f64,
    pub eval_pairs: u64,
    pub eval_lanes: usize,
    pub val_norm_tokens_sha256: String,
    pub val_raw_bytes_used: usize,
    pub val_chars_normalized: usize,
}

/// Read the committed CUSTOM_distill arm record.
pub fn load_committed_provenance(repo_root: &Path) -> Result<CommittedProvenance, OneTokenError> {
    let path = repo_root.join(D192_REAL_ARM_JSON);
    let bytes = fs::read(&path).map_err(|e| OneTokenError::Io {
        path: path.clone(),
        reason: e.to_string(),
    })?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| OneTokenError::Manifest {
            reason: format!("{}: {e}", path.display()),
        })?;
    let f = |ptr: &str| -> Result<f64, OneTokenError> {
        v.pointer(ptr)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| OneTokenError::Manifest {
                reason: format!("{} missing {ptr}", path.display()),
            })
    };
    let s = |ptr: &str| -> Result<String, OneTokenError> {
        v.pointer(ptr)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| OneTokenError::Manifest {
                reason: format!("{} missing {ptr}", path.display()),
            })
    };
    Ok(CommittedProvenance {
        arm_json: D192_REAL_ARM_JSON.to_string(),
        arm: s("/arm")?,
        ternary_val_bpc_per_normalized_char: f("/measurement/ternary_val_bpc_per_normalized_char")?,
        ternary_val_bits_per_raw_byte: f("/measurement/ternary_val_bits_per_raw_byte")?,
        eval_pairs: f("/measurement/eval_pairs")? as u64,
        eval_lanes: f("/measurement/eval_lanes")? as usize,
        val_norm_tokens_sha256: s("/corpus/val_norm_tokens_sha256")?,
        val_raw_bytes_used: f("/corpus/val_raw_bytes_used")? as usize,
        val_chars_normalized: f("/corpus/val_chars_normalized")? as usize,
    })
}

// ---------------------------------------------------------------------------
// fidelity (full committed pair set, lanes evaluated in parallel)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct D192RealFidelityReport {
    pub val_chars_total: usize,
    pub val_norm_tokens_sha256: String,
    /// The locally assembled normalized val stream is byte-identical to the
    /// one the committed measurement scored.
    pub val_sha_matches_committed: bool,
    pub eval_lanes: usize,
    pub eval_positions: usize,
    pub committed_eval_pairs: u64,
    /// True when this run scored exactly the committed pair set (same
    /// stream, same lane layout, all pairs).
    pub scored_committed_pair_set_exactly: bool,
    /// f32 trainer-port (hard ternary + Q8.8 scales + act fake-quant)
    /// bits/char over the scored positions.
    pub f32_port_val_bpc: f64,
    pub committed_ternary_val_bpc: f64,
    pub f32_port_delta_vs_committed: f64,
    pub f32_port_tolerance: f64,
    pub f32_port_reproduces_committed: bool,
    /// Canonical integer semantics bits/char over the same positions.
    pub int_val_bpc: f64,
    pub int_minus_f32_bpc: f64,
    /// bpc * (val_chars_total / val_raw_bytes_total): the committed
    /// bits/raw-byte re-expression, valid for the full pair set.
    pub int_val_bits_per_raw_byte: f64,
    pub f32_port_val_bits_per_raw_byte: f64,
    pub committed_ternary_val_bits_per_raw_byte: f64,
    /// Per-position argmax agreement (int vs f32, each carrying its own
    /// state).
    pub argmax_agreement: f64,
    pub int_stats: StateIntStatsReport,
}

struct LaneOut {
    bits_f: f64,
    bits_i: f64,
    agree: u64,
    stats: StateForwardStats,
}

/// Score the sequential charset val stream in the committed lane layout,
/// integer evaluator vs the f32 trainer port, lanes in parallel threads
/// (each lane's state carried from zero across its contiguous segment; the
/// per-lane accumulation order is exactly the sequential one).
pub fn run_d192_real_fidelity(
    repo_root: &Path,
    committed: &CommittedProvenance,
    ck: &StateCheckpoint,
    lowered: &IntStateLoweredModel,
    max_positions_per_lane: usize,
) -> Result<D192RealFidelityReport, OneTokenError> {
    let (ids, ids_sha) = build_val_char_ids(repo_root, committed.val_raw_bytes_used)?;
    let sha_ok = ids_sha == committed.val_norm_tokens_sha256;

    let lanes = committed.eval_lanes.max(1);
    let lane_len = ids.len() / lanes;
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
    let step = lowered.logit_dequant_step();
    let ln2 = std::f64::consts::LN_2;

    let lane_outs: Vec<LaneOut> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..lanes)
            .map(|lane| {
                let ids = &ids;
                scope.spawn(move || {
                    let base = lane * lane_len;
                    let mut f_state = vec![0.0f32; ck.topology().state_slots];
                    let mut i_state = lowered.zero_state();
                    let mut out = LaneOut {
                        bits_f: 0.0,
                        bits_i: 0.0,
                        agree: 0,
                        stats: StateForwardStats::new(),
                    };
                    for t in 0..pairs_per_lane {
                        let ctx = ids[base + t];
                        let tgt = usize::from(ids[base + t + 1]);

                        let f_logits =
                            gbf_kernel::state_model_ref::f32_state_forward(ck, ctx, &mut f_state);
                        let f64_logits: Vec<f64> = f_logits.iter().map(|l| f64::from(*l)).collect();
                        let f_lp = log_softmax_v(&f64_logits);
                        out.bits_f += -f_lp[tgt] / ln2;
                        let f_arg = argmax_v(&f64_logits);

                        let trace = lowered.forward(ctx, &mut i_state);
                        out.stats.merge(&trace.stats);
                        let i_logits: Vec<f64> =
                            trace.logits.iter().map(|l| f64::from(*l) * step).collect();
                        let i_lp = log_softmax_v(&i_logits);
                        out.bits_i += -i_lp[tgt] / ln2;
                        if f_arg == trace.argmax {
                            out.agree += 1;
                        }
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("fidelity lane thread"))
            .collect()
    });

    let positions = pairs_per_lane * lanes;
    let mut bits_f = 0.0f64;
    let mut bits_i = 0.0f64;
    let mut agree = 0u64;
    let mut merged = StateForwardStats::new();
    for lane in &lane_outs {
        bits_f += lane.bits_f;
        bits_i += lane.bits_i;
        agree += lane.agree;
        merged.merge(&lane.stats);
    }
    let f32_bpc = bits_f / positions as f64;
    let int_bpc = bits_i / positions as f64;
    let delta = (f32_bpc - committed.ternary_val_bpc_per_normalized_char).abs();
    let exact_pair_set =
        sha_ok && max_positions_per_lane == 0 && positions as u64 == committed.eval_pairs;
    let raw_byte_factor =
        committed.val_chars_normalized as f64 / committed.val_raw_bytes_used as f64;

    Ok(D192RealFidelityReport {
        val_chars_total: ids.len(),
        val_norm_tokens_sha256: ids_sha,
        val_sha_matches_committed: sha_ok,
        eval_lanes: lanes,
        eval_positions: positions,
        committed_eval_pairs: committed.eval_pairs,
        scored_committed_pair_set_exactly: exact_pair_set,
        f32_port_val_bpc: f32_bpc,
        committed_ternary_val_bpc: committed.ternary_val_bpc_per_normalized_char,
        f32_port_delta_vs_committed: delta,
        f32_port_tolerance: F32_PORT_TOLERANCE,
        f32_port_reproduces_committed: delta <= F32_PORT_TOLERANCE,
        int_val_bpc: int_bpc,
        int_minus_f32_bpc: int_bpc - f32_bpc,
        int_val_bits_per_raw_byte: int_bpc * raw_byte_factor,
        f32_port_val_bits_per_raw_byte: f32_bpc * raw_byte_factor,
        committed_ternary_val_bits_per_raw_byte: committed.ternary_val_bits_per_raw_byte,
        argmax_agreement: agree as f64 / positions as f64,
        int_stats: StateIntStatsReport::from_stats(&merged),
    })
}

// ---------------------------------------------------------------------------
// down-delta magnitude measurement (bd-2vkqt)
// ---------------------------------------------------------------------------

/// Measured (and structural) distribution of the wide down-projection delta
/// magnitudes on the REAL checkpoint (`down_delta_probe.v1`): the evidence
/// base for choosing the delta carrier width from data, not guesses.
#[derive(Debug, Clone, Serialize)]
pub struct DownDeltaProbeReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub export_dir: String,
    pub git_sha: String,
    /// Positions scored (int evaluator only, committed lane layout).
    pub eval_positions: usize,
    pub eval_lanes: usize,
    /// Down-projection deltas observed (positions * n_blocks * d_model).
    pub deltas_recorded: u64,
    /// Exact max unclamped |delta| (raw Q19.5; 32 raw = 1 real unit).
    pub max_abs_down_delta_raw: u64,
    pub max_abs_down_delta_units: f64,
    /// Histogram-derived quantile upper bounds (raw Q19.5, exact to one
    /// 32-raw bucket).
    pub p99_raw: u64,
    pub p999_raw: u64,
    pub p9999_raw: u64,
    /// Deltas at or above the Q11.5-era u16 carrier cap (65536 raw).
    pub count_at_or_above_65536: u64,
    /// Deltas that would escape a signed-i24 delta carrier (2^23 raw).
    pub count_at_or_above_i24: u64,
    /// Structural worst case from the actual weights/scales (no input can
    /// exceed this).
    pub structural_delta_bound_raw: u64,
    /// The i24 carrier ceiling the structural bound is compared against.
    pub i24_delta_bound: u64,
    pub structural_bound_fits_i24: bool,
    /// Same structural bound for the committed arm-B checkpoint (context for
    /// the per-checkpoint width decision).
    pub arm_b_structural_delta_bound_raw: Option<u64>,
    pub int_stats: StateIntStatsReport,
}

/// Run the int evaluator over the committed lane layout recording every
/// unclamped down-delta magnitude. `max_positions_per_lane = 0` scores the
/// full committed pair set.
pub fn run_down_delta_probe(
    repo_root: &Path,
    max_positions_per_lane: usize,
) -> Result<DownDeltaProbeReport, OneTokenError> {
    use gbf_kernel::state_model_ref::DownDeltaProbe;

    let committed = load_committed_provenance(repo_root)?;
    let bundle = load_state_checkpoint(&repo_root.join(D192_REAL_EXPORT_DIR))?;
    let ck = bundle.checkpoint;
    let lowered =
        IntStateLoweredModel::lower(&ck).map_err(|e| OneTokenError::Model(e.to_string()))?;
    let (ids, _) = build_val_char_ids(repo_root, committed.val_raw_bytes_used)?;

    let lanes = committed.eval_lanes.max(1);
    let lane_len = ids.len() / lanes;
    let pairs_per_lane = if max_positions_per_lane == 0 {
        lane_len - 1
    } else {
        (lane_len - 1).min(max_positions_per_lane)
    };

    let (probe, stats) = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..lanes)
            .map(|lane| {
                let ids = &ids;
                let lowered = &lowered;
                scope.spawn(move || {
                    let base = lane * lane_len;
                    let mut state = lowered.zero_state();
                    let mut probe = DownDeltaProbe::new();
                    let mut stats = StateForwardStats::new();
                    for t in 0..pairs_per_lane {
                        let trace =
                            lowered.forward_probed(ids[base + t], &mut state, Some(&mut probe));
                        stats.merge(&trace.stats);
                    }
                    (probe, stats)
                })
            })
            .collect();
        let mut probe = DownDeltaProbe::new();
        let mut stats = StateForwardStats::new();
        for h in handles {
            let (p, s) = h.join().expect("probe lane thread");
            probe.merge(&p);
            stats.merge(&s);
        }
        (probe, stats)
    });

    let arm_b_bound = load_state_checkpoint(&repo_root.join(crate::stateful::STATE_EXPORT_DIR))
        .ok()
        .and_then(|b| IntStateLoweredModel::lower(&b.checkpoint).ok())
        .map(|l| l.down_delta_structural_bound);
    let structural = lowered.down_delta_structural_bound;
    let i24_bound = gbf_kernel::state_model_ref::DOWN_DELTA_WIDE_BOUND;

    #[allow(clippy::cast_precision_loss)]
    Ok(DownDeltaProbeReport {
        schema: "down_delta_probe.v1",
        bead: "bd-2vkqt",
        export_dir: D192_REAL_EXPORT_DIR.to_string(),
        git_sha: git_head(repo_root),
        eval_positions: pairs_per_lane * lanes,
        eval_lanes: lanes,
        deltas_recorded: probe.total(),
        max_abs_down_delta_raw: probe.max_abs(),
        max_abs_down_delta_units: probe.max_abs() as f64 / 32.0,
        p99_raw: probe.quantile_upper_bound(0.99),
        p999_raw: probe.quantile_upper_bound(0.999),
        p9999_raw: probe.quantile_upper_bound(0.9999),
        count_at_or_above_65536: probe.count_at_or_above(65536),
        count_at_or_above_i24: probe.count_at_or_above(1 << 23),
        structural_delta_bound_raw: structural,
        i24_delta_bound: i24_bound,
        structural_bound_fits_i24: structural <= i24_bound,
        arm_b_structural_delta_bound_raw: arm_b_bound,
        int_stats: StateIntStatsReport::from_stats(&stats),
    })
}

// ---------------------------------------------------------------------------
// ROM / cycle / sparsity facts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct D192RealRomFacts {
    pub variant: String,
    pub rom_bytes: usize,
    pub bank_count: u16,
    pub uses_romb1_9bit_banking: bool,
    pub driver_bytes: usize,
    pub driver_bank_capacity: usize,
    pub driver_headroom_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
}

impl D192RealRomFacts {
    fn new(
        variant: &str,
        rom_bytes: usize,
        bank_count: u16,
        driver_bytes: usize,
        weight_code_bytes: usize,
        weight_chunk_count: usize,
        table_bytes: usize,
    ) -> Self {
        Self {
            variant: variant.to_string(),
            rom_bytes,
            bank_count,
            uses_romb1_9bit_banking: bank_count > 256,
            driver_bytes,
            driver_bank_capacity: STATE_DRIVER_BANK_CAPACITY,
            driver_headroom_bytes: STATE_DRIVER_BANK_CAPACITY.saturating_sub(driver_bytes),
            weight_code_bytes,
            weight_chunk_count,
            table_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct D192RealCycleFacts {
    pub macs_per_token: u64,
    pub one_token_mean_m_cycles: u64,
    pub multi_token_mean_m_cycles: u64,
    pub seconds_per_token_dmg: f64,
    /// One token is one charset char, so this equals seconds/token.
    pub seconds_per_char_dmg: f64,
    /// The synthetic d192-readiness generation-loop prediction, read from
    /// the committed readiness report (None if unavailable).
    pub synthetic_readiness_mean_m_cycles: Option<u64>,
    pub real_over_synthetic: Option<f64>,
    /// Ternary zero fraction of the real weights (V3 chunks skip zero
    /// weights, so sparsity drives cycles).
    pub real_ternary_zero_fraction: f64,
    /// Zero fraction of the synthetic readiness model, for the comparison.
    pub synthetic_ternary_zero_fraction: f64,
}

fn layer_zero_counts(layer: &TernaryLayer) -> (u64, u64) {
    let mut total = 0u64;
    let mut zeros = 0u64;
    for row in 0..layer.rows() {
        for &w in layer.row(row) {
            total += 1;
            if w == 0 {
                zeros += 1;
            }
        }
    }
    (total, zeros)
}

/// (total ternary weights, zero fraction) over every ternary matrix.
#[must_use]
pub fn ternary_zero_fraction(ck: &StateCheckpoint) -> (u64, f64) {
    let mut total = 0u64;
    let mut zeros = 0u64;
    let mut add = |t: (u64, u64)| {
        total += t.0;
        zeros += t.1;
    };
    add(layer_zero_counts(&ck.state_in));
    add(layer_zero_counts(&ck.state_out));
    for block in ck.blocks() {
        let (up, down) = block
            .as_dense()
            .expect("ternary_zero_fraction handles only dense checkpoints");
        add(layer_zero_counts(up));
        add(layer_zero_counts(down));
    }
    (total, zeros as f64 / total.max(1) as f64)
}

// ---------------------------------------------------------------------------
// shell gate summary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct D192ShellFacts {
    pub sampler: SamplerSettingFacts,
    pub prompt: String,
    pub rom: D192RealRomFacts,
    pub ui_bank_bytes: usize,
    pub n_gen_tokens: u8,
    pub boot_chrome_ok: bool,
    pub prompt_echo_ok: bool,
    pub sequences_match: bool,
    pub transcript_bg_ok: bool,
    pub post_run_chrome_ok: bool,
    pub returned_to_idle: bool,
    pub all_gates_pass: bool,
    pub n_tokens_generated: usize,
    pub determinism_sessions: usize,
    pub determinism_sequences_identical: bool,
    pub determinism_framebuffer_hashes_identical: bool,
    pub mean_m_cycles_per_token_boundary: u64,
    pub seconds_per_token_dmg: f64,
    pub mean_m_cycles_per_warmup_char: u64,
    pub fb_sha256_after_boot: String,
    pub fb_sha256_after_typing: String,
    pub fb_sha256_after_generation: String,
    #[serde(skip)]
    pub framebuffers: Vec<(String, Framebuffer)>,
    #[serde(skip)]
    pub transcript: String,
}

impl D192ShellFacts {
    #[must_use]
    pub fn gates_pass(&self) -> bool {
        self.all_gates_pass
            && self.determinism_sequences_identical
            && self.determinism_framebuffer_hashes_identical
    }
}

// ---------------------------------------------------------------------------
// qualitative samples (host 512 chars, ROM-verified prefix via the shell)
// ---------------------------------------------------------------------------

/// Sample settings `(temperature, top_k, rng_seed, prompt)`. Prompts must
/// fit the 20-char shell prompt row.
pub const D192_SAMPLE_SETTINGS: [(f64, u8, u16, &str); 3] = [
    (0.8, 8, 0x5EED, "The machines dreamed"),
    (0.6, 4, 0xBEEF, "In the beginning of"),
    (1.0, 8, 0xC0DE, "Once upon a midnight"),
];

/// Host mirror of a shell run without the transcript stop rule: warm up on
/// the prompt (no draws), then sample-feedback for `n_chars` tokens. The
/// first draws are identical to the shell ROM's, so a passing shell session
/// with the same `(cfg, prompt, rng_seed)` byte-verifies this sequence's
/// prefix.
#[must_use]
pub fn host_prompt_sample_generate(
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    prompt_ids: &[u8],
    rng_seed: u16,
    n_chars: usize,
) -> Vec<u8> {
    assert!(!prompt_ids.is_empty(), "prompt must be nonempty");
    let mut rng = XorShift16::new(rng_seed);
    let mut state = lowered.zero_state();
    let mut trace = None;
    for &c in prompt_ids {
        trace = Some(lowered.forward(c, &mut state));
    }
    let mut trace = trace.expect("prompt is nonempty");
    let mut sequence = Vec::with_capacity(n_chars);
    while sequence.len() < n_chars {
        let pick = sample_topk_trace(&trace.logits, cfg, &mut rng).picked as u8;
        sequence.push(pick);
        if sequence.len() < n_chars {
            trace = lowered.forward(pick, &mut state);
        }
    }
    sequence
}

#[derive(Debug, Clone, Serialize)]
pub struct D192RealSample {
    pub file: String,
    pub prompt: String,
    pub setting: SamplerSettingFacts,
    pub rng_seed: u16,
    pub n_chars: usize,
    /// Chars byte-verified on-device by the scripted shell session (typed
    /// prompt, START, generation) before the host continues the stream.
    pub rom_verified_prefix_chars: usize,
    pub rom_prefix_matches_host: bool,
    pub shell_session_gates_pass: bool,
    pub sequence_sha256: String,
    /// Generated text (prompt excluded).
    #[serde(skip)]
    pub text: String,
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PhaseStatus {
    pub phase: String,
    /// "pass", "fail", or "skipped".
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct D192RealReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub checkpoint_bead: &'static str,
    /// Canonical stateful integer semantics version this run implements
    /// (`state-int-semantics.v2`: clamp-free i24 down-delta carrier on the
    /// wide path, bd-2vkqt).
    pub int_semantics_version: &'static str,
    pub git_sha: String,
    pub export_dir: &'static str,
    pub options: D192RealOptions,
    pub committed: Option<CommittedProvenance>,
    pub phases: Vec<PhaseStatus>,
    pub all_gates_pass: bool,
    pub checkpoint: Option<StateCheckpointFacts>,
    pub topology: Option<TopologyFacts>,
    pub width: Option<D192WidthFacts>,
    pub layout: Option<D192LayoutFacts>,
    pub roms: Vec<D192RealRomFacts>,
    pub one_token_gate: Option<StateRomGateReport>,
    pub multi_token: Option<D192MultiTokenReport>,
    pub fidelity: Option<D192RealFidelityReport>,
    pub cycles: Option<D192RealCycleFacts>,
    pub shell: Option<D192ShellFacts>,
    pub samples: Vec<D192RealSample>,
    pub int_semantics_divergences: Vec<String>,
    pub caveats: Vec<String>,
}

impl D192RealReport {
    fn record_pass(&mut self, phase: &str) {
        self.phases.push(PhaseStatus {
            phase: phase.to_string(),
            status: "pass".to_string(),
            error: None,
        });
    }

    fn record_fail(&mut self, phase: &str, error: &OneTokenError) {
        self.phases.push(PhaseStatus {
            phase: phase.to_string(),
            status: "fail".to_string(),
            error: Some(error.to_string()),
        });
    }

    fn record_gate(&mut self, phase: &str, pass: bool, detail: Option<String>) {
        self.phases.push(PhaseStatus {
            phase: phase.to_string(),
            status: if pass { "pass" } else { "fail" }.to_string(),
            error: if pass { None } else { detail },
        });
    }

    fn record_skipped(&mut self, phase: &str) {
        self.phases.push(PhaseStatus {
            phase: phase.to_string(),
            status: "skipped".to_string(),
            error: None,
        });
    }

    fn phases_all_pass(&self) -> bool {
        !self.phases.is_empty() && self.phases.iter().all(|p| p.status == "pass")
    }
}

fn git_head(repo_root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn readiness_predicted_cycles(repo_root: &Path) -> Option<u64> {
    let bytes = fs::read(repo_root.join(D192_READINESS_REPORT)).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.pointer("/cycles/multi_token_mean_m_cycles")?.as_u64()
}

/// Run every phase against the REAL committed export, recording failures
/// honestly and skipping downstream phases past the first failure.
pub fn run_d192_real_bringup(repo_root: &Path, opts: &D192RealOptions) -> D192RealReport {
    let mut report = run_phases(repo_root, opts);
    finish_caveats(&mut report);
    report
}

/// V2 dispatch-lowering parity on the REAL committed d192 checkpoint (step 5,
/// docs/design/v2-dispatch-stateful.md): the V2 ROM must reproduce the
/// canonical integer semantics byte-for-byte (one-token + on-device
/// generation), and the sampling/shell driver variants must fit bank 0. Also
/// reports the V2-vs-V3 cycle and ROM-capacity accounting.
#[derive(Debug, Clone, Serialize)]
pub struct D192RealV2Parity {
    pub one_token_byte_exact: bool,
    pub multi_token_sequences_match: bool,
    pub multi_token_checkpoints_byte_exact: bool,
    pub v2_mean_m_cycles: u64,
    pub v3_mean_m_cycles: u64,
    pub v2_over_v3_cycles: f64,
    pub v2_seconds_per_token_dmg: f64,
    pub v2_bank_count: u16,
    pub v3_bank_count: u16,
    pub v2_rom_mib: f64,
    pub v3_rom_mib: f64,
    pub v2_weight_banks: usize,
    pub v3_weight_banks: usize,
    pub v2_driver_bytes: usize,
    pub v2_sampling_driver_bytes: usize,
    pub v2_sampling_fits_bank0: bool,
    pub v2_shell_driver_bytes: usize,
    pub v2_shell_fits_bank0: bool,
}

impl D192RealV2Parity {
    #[must_use]
    pub fn pass(&self) -> bool {
        self.one_token_byte_exact
            && self.multi_token_sequences_match
            && self.multi_token_checkpoints_byte_exact
            && self.v2_bank_count <= 512
            && self.v2_sampling_fits_bank0
            && self.v2_shell_fits_bank0
    }
}

/// Run the V2 parity gate on the real committed checkpoint. Heavy (emulates
/// the ~400-bank real d192 ROM under both lowerings), so it lives in the
/// `d192-real` bin, not the fast `--lib` suite.
pub fn run_d192_real_v2_parity(repo_root: &Path) -> Result<D192RealV2Parity, OneTokenError> {
    let bundle = load_state_checkpoint(&repo_root.join(D192_REAL_EXPORT_DIR))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;

    // One-token cases: zero state + a carried state from a short host stream.
    let mut cases: Vec<(usize, u8, Vec<i32>)> = vec![(0, 19u8, lowered.zero_state())];
    let mut state = lowered.zero_state();
    let mut input = 19u8;
    for pos in 1..=5usize {
        let trace = lowered.forward(input, &mut state);
        input = trace.argmax;
        if pos == 5 {
            cases.push((pos, input, state.clone()));
        }
    }
    let v2_gate = run_state_rom_gate_lowered(&lowered, &cases, WeightLowering::V2Dispatch)?;
    let v3_gate = run_state_rom_gate_lowered(&lowered, &cases, WeightLowering::V3)?;

    // On-device generation under V2 vs the host feedback loop.
    let mt = build_state_multi_token_rom_lowered(&lowered, 8, WeightLowering::V2Dispatch)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let run = run_state_seed_generation(&mt, &lowered, STATE_GENERATION_SEEDS[0])?;

    // Sampling + shell driver variants must also fit bank 0 under V2.
    let cfg = SamplerConfig::new(8, 2253).map_err(|e| OneTokenError::Rom(format!("{e:?}")))?;
    let samp = build_state_multi_token_sampling_rom_lowered(
        &lowered,
        32,
        &cfg,
        WeightLowering::V2Dispatch,
    )
    .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let font = synthetic_font_tiles();
    let shell = build_state_shell_rom_lowered(&lowered, &cfg, 8, &font, WeightLowering::V2Dispatch)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;

    // V3 capacity reference (the real d192 does fit V3, ~400 banks / 8 MiB).
    let v3 = build_state_one_token_rom(&lowered).map_err(|e| OneTokenError::Rom(e.to_string()))?;

    let bank0 = 0x4000 - 0x150;
    let v2_m = v2_gate.mean_m_cycles;
    let v3_m = v3_gate.mean_m_cycles;
    Ok(D192RealV2Parity {
        one_token_byte_exact: v2_gate.all_byte_exact,
        multi_token_sequences_match: run.sequences_match,
        multi_token_checkpoints_byte_exact: run.first_token_checkpoints_byte_exact
            && run.last_token_checkpoints_byte_exact,
        v2_mean_m_cycles: v2_m,
        v3_mean_m_cycles: v3_m,
        v2_over_v3_cycles: if v3_m == 0 {
            0.0
        } else {
            v2_m as f64 / v3_m as f64
        },
        v2_seconds_per_token_dmg: v2_m as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        v2_bank_count: v2_gate.rom.bank_count,
        v3_bank_count: v3.bank_count,
        v2_rom_mib: v2_gate.rom.rom_bytes as f64 / (1024.0 * 1024.0),
        v3_rom_mib: v3.rom.len() as f64 / (1024.0 * 1024.0),
        v2_weight_banks: v2_gate.rom.weight_chunk_count,
        v3_weight_banks: v3.weight_chunk_count,
        v2_driver_bytes: v2_gate.rom.driver_bytes,
        v2_sampling_driver_bytes: samp.driver_bytes,
        v2_sampling_fits_bank0: samp.driver_bytes < bank0,
        v2_shell_driver_bytes: shell.driver_bytes,
        v2_shell_fits_bank0: shell.driver_bytes < bank0,
    })
}

#[allow(clippy::too_many_lines)]
fn run_phases(repo_root: &Path, opts: &D192RealOptions) -> D192RealReport {
    let mut report = D192RealReport {
        schema: "d192_real_bringup.v2",
        bead: "bd-pp43d",
        checkpoint_bead: "bd-3771m",
        int_semantics_version: gbf_kernel::state_model_ref::STATE_INT_SEMANTICS_VERSION,
        git_sha: git_head(repo_root),
        export_dir: D192_REAL_EXPORT_DIR,
        options: opts.clone(),
        committed: None,
        phases: Vec::new(),
        all_gates_pass: false,
        checkpoint: None,
        topology: None,
        width: None,
        layout: None,
        roms: Vec::new(),
        one_token_gate: None,
        multi_token: None,
        fidelity: None,
        cycles: None,
        shell: None,
        samples: Vec::new(),
        int_semantics_divergences: STATE_INT_SEMANTIC_DIVERGENCES
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        caveats: Vec::new(),
    };

    let remaining_phases = |report: &mut D192RealReport, from: usize| {
        const PHASES: [&str; 10] = [
            "committed-provenance",
            "load-real-checkpoint",
            "integer-lowering",
            "wram-layout",
            "one-token-gate",
            "multi-token-gate",
            "fidelity",
            "cycles",
            "shell-session-gate",
            "samples",
        ];
        for phase in &PHASES[from..] {
            report.record_skipped(phase);
        }
    };

    // Phase 0: committed provenance.
    let committed = match load_committed_provenance(repo_root) {
        Ok(c) => {
            report.record_pass("committed-provenance");
            report.committed = Some(c.clone());
            c
        }
        Err(e) => {
            report.record_fail("committed-provenance", &e);
            remaining_phases(&mut report, 1);
            return report;
        }
    };

    // Phase 1: production loader (manifest topology + per-tensor sha256).
    let bundle = match load_state_checkpoint(&repo_root.join(D192_REAL_EXPORT_DIR)) {
        Ok(b) => {
            report.record_pass("load-real-checkpoint");
            b
        }
        Err(e) => {
            let error = OneTokenError::from(e);
            report.record_fail("load-real-checkpoint", &error);
            remaining_phases(&mut report, 2);
            return report;
        }
    };
    report.checkpoint = Some(StateCheckpointFacts {
        export_dir: D192_REAL_EXPORT_DIR.to_string(),
        manifest_schema: bundle.manifest_schema.clone(),
        manifest_sha256: bundle.manifest_sha256.clone(),
        trainer_git_sha: bundle.manifest_git_sha.clone(),
        tensors_verified_sha256: bundle.tensors_verified,
    });
    let topology = bundle.topology;
    report.topology = Some(TopologyFacts {
        d_model: topology.d_model,
        d_ff: topology.d_ff,
        n_blocks: topology.n_blocks,
        state_slots: topology.state_slots,
        vocab: topology.vocab,
    });
    let ck = bundle.checkpoint;

    // Phase 2: integer lowering (DownEpilogueOverflow fires here if the
    // real scale/accumulator products break the u32 epilogue bound).
    let lowered = match IntStateLoweredModel::lower(&ck) {
        Ok(l) => {
            report.record_pass("integer-lowering");
            l
        }
        Err(e) => {
            let err = OneTokenError::Model(e.to_string());
            report.record_fail("integer-lowering", &err);
            remaining_phases(&mut report, 3);
            return report;
        }
    };
    report.width = Some(D192WidthFacts {
        down_acc_width: format!("{:?}", lowered.down_width),
        down_acc_structural_bound: lowered.down_acc_structural_bound,
        i16_bound: 32767,
        down_delta_structural_bound: lowered.down_delta_structural_bound,
        i24_delta_bound: gbf_kernel::state_model_ref::DOWN_DELTA_WIDE_BOUND,
        decision_source: "structural per-row worst case over the actual ternary weights \
                          (the f_s5_state_checkpoint_export.v1 manifest declares no measured \
                          activation ranges, so lowering never relies on unmeasured statistics)",
    });

    // Phase 3: WRAM layout (budget asserted; WramOverflow fires here).
    match StateWramLayout::plan(topology, lowered.down_width, false) {
        Ok(layout) => {
            report.record_pass("wram-layout");
            report.layout = Some(D192LayoutFacts {
                wram_bytes_allocated: layout.bytes_allocated,
                wram_budget_bytes: 8192,
                per_block_residual_dumps_kept: layout.xdump.is_some(),
                out_acc_dump_kept: layout.sacc_separate,
                scratch_overlaid_on_matvec_arena: layout.absx == layout.acc,
                untouched_regions: layout.untouched_regions(),
            });
        }
        Err(e) => {
            let err = OneTokenError::Rom(e.to_string());
            report.record_fail("wram-layout", &err);
            remaining_phases(&mut report, 4);
            return report;
        }
    }

    // Phase 4: one-token gate over harvested (input, carried state) cases
    // (the ROM build runs inside; DriverOverflowsBank0 fires here).
    let one_token = (|| -> Result<StateRomGateReport, OneTokenError> {
        let (ids, _) = build_val_char_ids(repo_root, committed.val_raw_bytes_used)?;
        let positions: Vec<usize> = if opts.quick_mode {
            vec![0, 1]
        } else {
            D192_REAL_ONE_TOKEN_POSITIONS.to_vec()
        };
        let cases = harvest_state_cases(&lowered, &ids, &positions);
        run_state_rom_gate(&lowered, &cases)
    })();
    let one_token = match one_token {
        Ok(gate) => {
            report.roms.push(D192RealRomFacts::new(
                "one-token",
                gate.rom.rom_bytes,
                gate.rom.bank_count,
                gate.rom.driver_bytes,
                gate.rom.weight_code_bytes,
                gate.rom.weight_chunk_count,
                gate.rom.table_bytes,
            ));
            report.record_gate(
                "one-token-gate",
                gate.all_byte_exact,
                Some(format!(
                    "{}/{} cases byte-exact",
                    gate.runs.iter().filter(|r| r.byte_exact).count(),
                    gate.runs.len()
                )),
            );
            let pass = gate.all_byte_exact;
            report.one_token_gate = Some(gate.clone());
            if !pass {
                remaining_phases(&mut report, 5);
                return report;
            }
            gate
        }
        Err(e) => {
            report.record_fail("one-token-gate", &e);
            remaining_phases(&mut report, 5);
            return report;
        }
    };

    // Phase 5: multi-token sustained on-device generation.
    let multi = (|| -> Result<D192MultiTokenReport, OneTokenError> {
        let rom = build_state_multi_token_rom(&lowered, opts.multi_token_tokens)
            .map_err(|e| OneTokenError::Rom(e.to_string()))?;
        report.roms.push(D192RealRomFacts::new(
            "multi-token",
            rom.rom.len(),
            rom.bank_count,
            rom.driver_bytes,
            rom.weight_code_bytes,
            rom.weight_chunk_count,
            rom.table_bytes,
        ));
        let mut runs = Vec::new();
        for &seed in &opts.multi_token_seeds {
            runs.push(run_state_seed_generation(&rom, &lowered, seed)?);
        }
        Ok(D192MultiTokenReport {
            n_tokens: opts.multi_token_tokens,
            seeds: opts.multi_token_seeds.clone(),
            all_sequences_match: runs.iter().all(|r| r.sequences_match),
            all_health_checks_pass: runs
                .iter()
                .all(crate::stateful::StateSeedRun::all_checks_pass),
            runs,
        })
    })();
    let multi = match multi {
        Ok(m) => {
            let pass = m.all_sequences_match && m.all_health_checks_pass;
            report.record_gate(
                "multi-token-gate",
                pass,
                Some(format!(
                    "sequences {} health {}",
                    m.all_sequences_match, m.all_health_checks_pass
                )),
            );
            report.multi_token = Some(m.clone());
            if !pass {
                remaining_phases(&mut report, 6);
                return report;
            }
            m
        }
        Err(e) => {
            report.record_fail("multi-token-gate", &e);
            remaining_phases(&mut report, 6);
            return report;
        }
    };

    // Phase 6: fidelity over the committed val pair set.
    match run_d192_real_fidelity(
        repo_root,
        &committed,
        &ck,
        &lowered,
        opts.fidelity_positions_per_lane,
    ) {
        Ok(f) => {
            // Quick mode scores a small subset, so the committed-value
            // comparison is statistically meaningless there; the evidence
            // configuration must score the exact committed pair set and
            // reproduce the committed value.
            let pass = f.val_sha_matches_committed
                && (opts.quick_mode
                    || (f.f32_port_reproduces_committed && f.scored_committed_pair_set_exactly));
            report.record_gate(
                "fidelity",
                pass,
                Some(format!(
                    "val sha match {}, f32 port |delta| {:.3e} vs tolerance {:.1e}, exact pair \
                     set {}",
                    f.val_sha_matches_committed,
                    f.f32_port_delta_vs_committed,
                    f.f32_port_tolerance,
                    f.scored_committed_pair_set_exactly
                )),
            );
            report.fidelity = Some(f);
            if !pass {
                remaining_phases(&mut report, 7);
                return report;
            }
        }
        Err(e) => {
            report.record_fail("fidelity", &e);
            remaining_phases(&mut report, 7);
            return report;
        }
    }

    // Phase 7: cycles (from the gates just run + the committed synthetic
    // prediction + real sparsity).
    {
        let multi_mean = multi.runs.iter().map(|r| r.cycles.mean).sum::<u64>()
            / u64::try_from(multi.runs.len().max(1)).expect("run count fits u64");
        let seconds = multi_mean as f64 / DMG_M_CYCLES_PER_SECOND as f64;
        let synthetic = readiness_predicted_cycles(repo_root);
        let (_, real_zero) = ternary_zero_fraction(&ck);
        let synthetic_ck = synthetic_state_checkpoint_with(topology, D192_SYNTHETIC_SEED);
        let (_, synth_zero) = ternary_zero_fraction(&synthetic_ck);
        report.cycles = Some(D192RealCycleFacts {
            macs_per_token: topology.macs_per_token(),
            one_token_mean_m_cycles: one_token.mean_m_cycles,
            multi_token_mean_m_cycles: multi_mean,
            seconds_per_token_dmg: seconds,
            seconds_per_char_dmg: seconds,
            synthetic_readiness_mean_m_cycles: synthetic,
            real_over_synthetic: synthetic.map(|s| multi_mean as f64 / s as f64),
            real_ternary_zero_fraction: real_zero,
            synthetic_ternary_zero_fraction: synth_zero,
        });
        report.record_pass("cycles");
    }

    // Phase 8: interactive shell scripted joypad session gate.
    match run_shell_bringup(repo_root, D192_REAL_EXPORT_DIR, opts.shell_gen_tokens) {
        Ok(shell_report) => {
            let s = &shell_report.session;
            let facts = D192ShellFacts {
                sampler: shell_report.sampler.clone(),
                prompt: shell_report.prompt_text.clone(),
                rom: D192RealRomFacts::new(
                    "shell",
                    shell_report.rom.rom_bytes,
                    shell_report.rom.bank_count,
                    shell_report.rom.driver_bytes,
                    shell_report.rom.weight_code_bytes,
                    shell_report.rom.weight_chunk_count,
                    shell_report.rom.table_bytes,
                ),
                ui_bank_bytes: shell_report.rom.ui_bank_bytes,
                n_gen_tokens: shell_report.rom.n_gen_tokens,
                boot_chrome_ok: s.boot_chrome_ok,
                prompt_echo_ok: s.prompt_echo_ok,
                sequences_match: s.sequences_match,
                transcript_bg_ok: s.transcript_bg_ok,
                post_run_chrome_ok: s.post_run_chrome_ok,
                returned_to_idle: s.returned_to_idle,
                all_gates_pass: s.all_gates_pass(),
                n_tokens_generated: s.n_tokens_generated,
                determinism_sessions: shell_report.determinism.sessions,
                determinism_sequences_identical: shell_report.determinism.sequences_identical,
                determinism_framebuffer_hashes_identical: shell_report
                    .determinism
                    .framebuffer_hashes_identical,
                mean_m_cycles_per_token_boundary: shell_report
                    .cadence
                    .mean_m_cycles_per_token_boundary,
                seconds_per_token_dmg: shell_report.cadence.seconds_per_token_dmg,
                mean_m_cycles_per_warmup_char: shell_report.cadence.mean_m_cycles_per_warmup_char,
                fb_sha256_after_boot: s.fb_sha256_after_boot.clone(),
                fb_sha256_after_typing: s.fb_sha256_after_typing.clone(),
                fb_sha256_after_generation: s.fb_sha256_after_generation.clone(),
                framebuffers: s.framebuffers.clone(),
                transcript: crate::shell::transcript_text(&s.prompt_ids, &s.rom_sequence),
            };
            let pass = facts.gates_pass();
            report.record_gate(
                "shell-session-gate",
                pass,
                Some(format!(
                    "session gates {} determinism {}",
                    facts.all_gates_pass,
                    facts.determinism_sequences_identical
                        && facts.determinism_framebuffer_hashes_identical
                )),
            );
            report.shell = Some(facts);
            if !pass {
                remaining_phases(&mut report, 9);
                return report;
            }
        }
        Err(e) => {
            report.record_fail("shell-session-gate", &e);
            remaining_phases(&mut report, 9);
            return report;
        }
    }

    // Phase 9: qualitative samples (host stream, ROM-verified prefix via a
    // dedicated shell session per sampler setting).
    let samples = (|| -> Result<Vec<D192RealSample>, OneTokenError> {
        let step = lowered.logit_dequant_step();
        let font = shell_font_tiles();
        let mut samples = Vec::new();
        for &(temperature, k, rng_seed, prompt) in &D192_SAMPLE_SETTINGS {
            let cfg = SamplerConfig::from_temperature(k, step, temperature)
                .map_err(|e| OneTokenError::Model(format!("sample sampler config: {e}")))?;
            let prompt_ids: Vec<u8> = prompt
                .chars()
                .map(|c| {
                    char_to_id(c).ok_or_else(|| {
                        OneTokenError::Model(format!("prompt char {c:?} not in charset_v1"))
                    })
                })
                .collect::<Result<_, _>>()?;
            let rom = build_state_shell_rom(&lowered, &cfg, opts.shell_gen_tokens, &font)
                .map_err(|e| OneTokenError::Rom(e.to_string()))?;
            let session = run_shell_session(&rom, &lowered, &cfg, &prompt_ids, rng_seed)?;
            let host = host_prompt_sample_generate(
                &lowered,
                &cfg,
                &prompt_ids,
                rng_seed,
                opts.sample_chars,
            );
            let prefix_len = session.rom_sequence.len().min(host.len());
            let prefix_ok = session.rom_sequence.len() <= host.len()
                && session.rom_sequence.as_slice() == &host[..prefix_len];
            samples.push(D192RealSample {
                file: format!(
                    "sample_T{}_k{}_rng_{:04x}.txt",
                    format!("{temperature:.2}").replace('.', "p"),
                    k,
                    rng_seed
                ),
                prompt: prompt.to_string(),
                setting: SamplerSettingFacts {
                    top_k: cfg.k(),
                    scale_q16: cfg.scale_q16(),
                    requested_temperature: temperature,
                    effective_temperature: cfg.effective_temperature(step),
                },
                rng_seed,
                n_chars: host.len(),
                rom_verified_prefix_chars: prefix_len,
                rom_prefix_matches_host: prefix_ok,
                shell_session_gates_pass: session.all_gates_pass(),
                sequence_sha256: sha256(&host).to_hex(),
                text: render_char_sample(&host),
            });
        }
        Ok(samples)
    })();
    match samples {
        Ok(samples) => {
            let pass = samples
                .iter()
                .all(|s| s.rom_prefix_matches_host && s.shell_session_gates_pass);
            report.record_gate(
                "samples",
                pass,
                Some(
                    samples
                        .iter()
                        .map(|s| {
                            format!(
                                "{}: prefix {} gates {}",
                                s.file, s.rom_prefix_matches_host, s.shell_session_gates_pass
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            );
            report.samples = samples;
        }
        Err(e) => {
            report.record_fail("samples", &e);
        }
    }

    report
}

fn finish_caveats(report: &mut D192RealReport) {
    report.all_gates_pass = report.phases_all_pass();
    report.caveats = vec![
        "All device timing is DMG M-cycle-accurate emulated time (gbf-emu headless), \
         interrupts disabled, SP repurposed inside weight chunks (bake-off convention); \
         production kernels pay yield/safe-point overhead on top."
            .to_string(),
        "Sample provenance: each sample's prefix is byte-verified on-device by a full \
         scripted shell session (typed prompt, START, generation to the transcript stop \
         rule); the remaining chars come from the same host integer evaluator + sampler + \
         RNG that produced the verified prefix. The output ring and transcript cap \
         on-device verification at ~200 tokens."
            .to_string(),
        "Stateful model: per-token integer/f32 rounding differences accumulate through the \
         carried state, so per-position fidelity deltas are structurally larger than dense \
         per-context numbers; the deployment-relevant number is the integer path's own val \
         bpc."
            .to_string(),
        "bits/raw-byte re-expresses bpc/normalized-char via the committed factor \
         val_chars_normalized / val_raw_bytes_used, the same method as the committed arm \
         record."
            .to_string(),
    ];
    if report.options.quick_mode {
        report.caveats.push(
            "QUICK MODE: gate sizes are development smoke sizes; this report is not \
             evidence."
                .to_string(),
        );
    }
}

// ---------------------------------------------------------------------------
// markdown
// ---------------------------------------------------------------------------

/// Render the README (generated, never hand-written).
#[must_use]
pub fn d192_real_report_to_markdown(r: &D192RealReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Real d192 checkpoint bring-up ({})", r.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "The REAL S8 distilled student ({}, `{}`) through the parameterized stateful ROM \
         pipeline that the synthetic d192-readiness gate pre-cleared. Generated by `cargo run \
         --release -p gbf-bench --bin d192-real`; every number is program output at git `{}`. \
         Canonical integer semantics: `{}` (the v1 run of this evidence clamped the wide \
         down-delta at 65535 raw and scored int 4.6803 bpc; see `FIDELITY-FIX.md`, bd-2vkqt).",
        r.checkpoint_bead, r.export_dir, r.git_sha, r.int_semantics_version
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Phases");
    let _ = writeln!(out);
    let _ = writeln!(out, "| phase | status | detail |");
    let _ = writeln!(out, "|---|---|---|");
    for p in &r.phases {
        let _ = writeln!(
            out,
            "| {} | **{}** | {} |",
            p.phase,
            p.status.to_uppercase(),
            p.error.as_deref().unwrap_or("")
        );
    }
    let _ = writeln!(out);

    if let (Some(c), Some(t)) = (&r.checkpoint, &r.topology) {
        let _ = writeln!(out, "## Checkpoint");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- `{}` ({}), manifest sha256 `{}`, trainer git `{}`, {} tensors sha256-verified \
             through the production loader",
            c.export_dir,
            c.manifest_schema,
            &c.manifest_sha256[..16],
            c.trainer_git_sha,
            c.tensors_verified_sha256
        );
        let _ = writeln!(
            out,
            "- Topology: d{} / ff{} / {} blocks / {} state slots / vocab {}",
            t.d_model, t.d_ff, t.n_blocks, t.state_slots, t.vocab
        );
        if let Some(committed) = &r.committed {
            let _ = writeln!(
                out,
                "- Committed trainer measurement (`{}`): hard-ternary val {:.6} bpc/char = \
                 {:.6} bits/raw-byte over {} pairs",
                committed.arm_json,
                committed.ternary_val_bpc_per_normalized_char,
                committed.ternary_val_bits_per_raw_byte,
                committed.eval_pairs
            );
        }
        let _ = writeln!(out);
    }

    if let Some(w) = &r.width {
        let _ = writeln!(out, "## Accumulator widths on the real weights");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- Down-projection accumulators: **{}** — structural per-row bound {} vs i16 bound \
             {}; decision source: {}",
            w.down_acc_width, w.down_acc_structural_bound, w.i16_bound, w.decision_source
        );
        let _ = writeln!(
            out,
            "- Down-delta carrier: structural per-row delta bound {} vs signed-i24 carrier \
             bound {} — the wide path carries the Q19.5 delta exactly (clamp-free, proven at \
             lowering; `DownDeltaEscapesI24` otherwise)",
            w.down_delta_structural_bound, w.i24_delta_bound
        );
        let _ = writeln!(out);
    }
    if let Some(l) = &r.layout {
        let _ = writeln!(out, "## WRAM budget");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- {} of {} bytes allocated; per-block residual dumps kept: {}; out-acc dump kept: \
             {}; |x|/out-acc scratch overlaid on the matvec arena: {}",
            l.wram_bytes_allocated,
            l.wram_budget_bytes,
            l.per_block_residual_dumps_kept,
            l.out_acc_dump_kept,
            l.scratch_overlaid_on_matvec_arena
        );
        let _ = writeln!(out);
    }
    if !r.roms.is_empty() {
        let _ = writeln!(out, "## ROMs (driver headroom is the known risk)");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| variant | ROM bytes | banks | ROMB1 9-bit | driver B | headroom B (of {}) | \
             weight code B | chunks | tables B |",
            STATE_DRIVER_BANK_CAPACITY
        );
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
        for rom in &r.roms {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                rom.variant,
                rom.rom_bytes,
                rom.bank_count,
                rom.uses_romb1_9bit_banking,
                rom.driver_bytes,
                rom.driver_headroom_bytes,
                rom.weight_code_bytes,
                rom.weight_chunk_count,
                rom.table_bytes
            );
        }
        if let Some(shell) = &r.shell {
            let _ = writeln!(
                out,
                "| shell (+UI bank {} B) | {} | {} | {} | {} | {} | {} | {} | {} |",
                shell.ui_bank_bytes,
                shell.rom.rom_bytes,
                shell.rom.bank_count,
                shell.rom.uses_romb1_9bit_banking,
                shell.rom.driver_bytes,
                shell.rom.driver_headroom_bytes,
                shell.rom.weight_code_bytes,
                shell.rom.weight_chunk_count,
                shell.rom.table_bytes
            );
        }
        let _ = writeln!(out);
    }

    if let Some(g) = &r.one_token_gate {
        let _ = writeln!(out, "## One-token gate (host-poked carried state)");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- **{}** — {}/{} (input, state) cases byte-exact across all layout WRAM checkpoints",
            if g.all_byte_exact { "PASS" } else { "FAIL" },
            g.runs.iter().filter(|run| run.byte_exact).count(),
            g.runs.len()
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| input id | state from val pos | zero state | host argmax | ROM argmax | \
             byte-exact | M-cycles |"
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
    }

    if let Some(m) = &r.multi_token {
        let _ = writeln!(out, "## Multi-token gate (on-device generation)");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- {} tokens per seed, seeds {:?}: sequences **{}**, health **{}**",
            m.n_tokens,
            m.seeds,
            if m.all_sequences_match {
                "PASS"
            } else {
                "FAIL"
            },
            if m.all_health_checks_pass {
                "PASS"
            } else {
                "FAIL"
            }
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| seed id | sequence match | first/last dumps | cycles min | median | max | \
             max/min | SP home | WRAM clean |"
        );
        let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
        for run in &m.runs {
            let _ = writeln!(
                out,
                "| {} | {} | {}/{} | {} | {} | {} | {:.5} | {} | {} |",
                run.seed_id,
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
            );
        }
        let _ = writeln!(out);
    }

    if let Some(f) = &r.fidelity {
        let _ = writeln!(out, "## Fidelity (integer vs trainer f32, state carried)");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- Val stream: {} charset_v1 chars, normalized-token sha256 `{}` — committed \
             stream match: **{}**",
            f.val_chars_total,
            &f.val_norm_tokens_sha256[..16],
            f.val_sha_matches_committed
        );
        let _ = writeln!(
            out,
            "- Sequential positions scored: {} ({} lanes; committed pair set scored exactly: \
             {})",
            f.eval_positions, f.eval_lanes, f.scored_committed_pair_set_exactly
        );
        let _ = writeln!(
            out,
            "- f32-port val bpc: {:.6} (committed {:.6}, |delta| {:.3e} vs tolerance {:.1e} — \
             port validation **{}**)",
            f.f32_port_val_bpc,
            f.committed_ternary_val_bpc,
            f.f32_port_delta_vs_committed,
            f.f32_port_tolerance,
            if f.f32_port_reproduces_committed {
                "PASS"
            } else {
                "FAIL"
            }
        );
        let _ = writeln!(
            out,
            "- **Integer-semantics val bpc: {:.6}** ({:+.6} vs the f32 port) = {:.6} \
             bits/raw-byte (committed hard-ternary: {:.6})",
            f.int_val_bpc,
            f.int_minus_f32_bpc,
            f.int_val_bits_per_raw_byte,
            f.committed_ternary_val_bits_per_raw_byte
        );
        let _ = writeln!(
            out,
            "- Per-position argmax agreement (int vs f32, each carrying its own state): {:.4}%",
            f.argmax_agreement * 100.0
        );
        let s = &f.int_stats;
        let _ = writeln!(
            out,
            "- Saturation/range on the real weights over all scored positions: max |in-proj \
             acc| {} / |FFN acc| {} (i16: {}), max |state| {} (i24 bound {}: {}, **{} clamp \
             events**), max |out acc| {} (i32: {}), max |residual| {} (**{} i24 wraps**), {} \
             down-delta clamps, {} y saturations, max |logit| {} (i24: {})",
            s.max_abs_in_acc,
            s.max_abs_matvec_acc,
            if s.matvec_acc_fits_i16 {
                "ok"
            } else {
                "VIOLATED"
            },
            s.max_abs_state,
            (1u32 << 23) - 1,
            if s.state_fits_i24 { "ok" } else { "SATURATED" },
            s.state_clamp_events,
            s.max_abs_out_acc,
            if s.out_acc_fits_i32 { "ok" } else { "VIOLATED" },
            s.max_abs_residual,
            s.residual_i24_wrap_events,
            s.down_delta_clamp_events,
            s.y_saturation_events,
            s.max_abs_logit,
            if s.logits_fit_i24 { "ok" } else { "VIOLATED" }
        );
        let _ = writeln!(out);
    }

    if let Some(c) = &r.cycles {
        let _ = writeln!(out, "## Cycles per token (real weights)");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- {} MACs/token; one-token mean {} M-cycles; generation-loop mean {} M-cycles = \
             **{:.3} s/token = {:.3} s/char** on DMG",
            c.macs_per_token,
            c.one_token_mean_m_cycles,
            c.multi_token_mean_m_cycles,
            c.seconds_per_token_dmg,
            c.seconds_per_char_dmg
        );
        if let (Some(pred), Some(ratio)) =
            (c.synthetic_readiness_mean_m_cycles, c.real_over_synthetic)
        {
            let _ = writeln!(
                out,
                "- Synthetic readiness prediction was {pred} M-cycles/token; real/synthetic = \
                 {ratio:.4} (real ternary zero fraction {:.4} vs synthetic {:.4}; V3 chunks \
                 skip zero weights)",
                c.real_ternary_zero_fraction, c.synthetic_ternary_zero_fraction
            );
        }
        let _ = writeln!(out);
    }

    if let Some(s) = &r.shell {
        let _ = writeln!(out, "## Interactive shell scripted joypad session");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- Prompt `{}` typed on the on-screen keyboard, START submits; top-k {} at T {} \
             (effective {:.4}); {} tokens generated on-device",
            s.prompt,
            s.sampler.top_k,
            s.sampler.requested_temperature,
            s.sampler.effective_temperature,
            s.n_tokens_generated
        );
        let _ = writeln!(
            out,
            "- Gates: boot chrome {}, prompt echo {}, sequence {}, transcript BG {}, post-run \
             chrome {}, idle return {} — **{}**",
            s.boot_chrome_ok,
            s.prompt_echo_ok,
            s.sequences_match,
            s.transcript_bg_ok,
            s.post_run_chrome_ok,
            s.returned_to_idle,
            if s.all_gates_pass { "PASS" } else { "FAIL" }
        );
        let _ = writeln!(
            out,
            "- Determinism over {} sessions: sequences identical {}, framebuffer hashes \
             identical {}",
            s.determinism_sessions,
            s.determinism_sequences_identical,
            s.determinism_framebuffer_hashes_identical
        );
        let _ = writeln!(
            out,
            "- Cadence: {} M-cycles/token boundary = {:.3} s/token; warmup {} M-cycles/prompt \
             char",
            s.mean_m_cycles_per_token_boundary,
            s.seconds_per_token_dmg,
            s.mean_m_cycles_per_warmup_char
        );
        let _ = writeln!(out);
    }

    if !r.samples.is_empty() {
        let _ = writeln!(out, "## Sampled text (best deployable model to date)");
        let _ = writeln!(out);
        for s in &r.samples {
            let _ = writeln!(
                out,
                "### `{}` — T {} (effective {:.4}), top-k {}, rng 0x{:04X}",
                s.file,
                s.setting.requested_temperature,
                s.setting.effective_temperature,
                s.setting.top_k,
                s.rng_seed
            );
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "Prompt `{}`; {} chars, first {} chars ROM-verified by a full scripted shell \
                 session (prefix match {}, session gates {}).",
                s.prompt,
                s.n_chars,
                s.rom_verified_prefix_chars,
                s.rom_prefix_matches_host,
                s.shell_session_gates_pass
            );
            let _ = writeln!(out);
            let _ = writeln!(out, "```text");
            let _ = writeln!(out, "{}{}", s.prompt, s.text);
            let _ = writeln!(out, "```");
            let _ = writeln!(out);
        }
    }

    let _ = writeln!(out, "## Documented integer-semantics divergences");
    let _ = writeln!(out);
    for d in &r.int_semantics_divergences {
        let _ = writeln!(out, "- {d}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Caveats");
    let _ = writeln!(out);
    for c in &r.caveats {
        let _ = writeln!(out, "- {c}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_kernel::state_model_ref::synthetic_state_checkpoint;

    /// The committed arm record must parse and carry the hard-ternary
    /// measurement the fidelity gate compares against.
    #[test]
    fn committed_provenance_parses_the_real_arm_record() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf();
        let c = load_committed_provenance(&repo_root).expect("arm json parses");
        assert_eq!(c.arm, "CUSTOM_distill");
        assert!(c.ternary_val_bpc_per_normalized_char > 2.0);
        assert!(c.ternary_val_bpc_per_normalized_char < 4.0);
        assert_eq!(c.eval_lanes, 8);
        assert_eq!(c.val_norm_tokens_sha256.len(), 64);
    }

    /// Step 5: the V2 dispatch lowering must be byte-exact on the REAL
    /// committed d192 checkpoint (one-token + on-device generation), and the
    /// sampling/shell driver variants must fit bank 0. Heavy (emulates the
    /// ~400-bank ROM under both lowerings), so it is `#[ignore]`d and exercised
    /// explicitly / by the `d192-real` bin.
    #[test]
    #[ignore = "slow real-checkpoint emulation under both lowerings; run explicitly"]
    fn v2_parity_byte_exact_on_real_checkpoint() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf();
        let p = run_d192_real_v2_parity(&repo_root).expect("real-checkpoint V2 parity runs");
        eprintln!("REAL-CKPT V2 PARITY: {p:#?}");
        assert!(p.one_token_byte_exact, "V2 one-token must be byte-exact");
        assert!(
            p.multi_token_sequences_match && p.multi_token_checkpoints_byte_exact,
            "V2 generation must be byte-exact"
        );
        assert!(p.v2_bank_count <= 512, "V2 real ROM must fit 512 banks");
        assert!(
            p.v2_sampling_fits_bank0,
            "V2 sampling driver must fit bank 0"
        );
        assert!(p.v2_shell_fits_bank0, "V2 shell driver must fit bank 0");
        assert!(p.pass());
    }

    /// The host prompt-sample stream's prefix must be exactly what a shell
    /// session generates for the same (cfg, prompt, rng): this is the
    /// ROM-verified-prefix provenance chain used for the real samples.
    #[test]
    fn host_prompt_sample_prefix_matches_shell_session_on_synthetic_model() {
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let step = lowered.logit_dequant_step();
        let cfg = SamplerConfig::from_temperature(8, step, 0.8).expect("valid sampler");
        let font = shell_font_tiles();
        let rom = build_state_shell_rom(&lowered, &cfg, 6, &font).expect("builds");
        let prompt_ids: Vec<u8> = "Hi"
            .chars()
            .map(|c| char_to_id(c).expect("printable"))
            .collect();
        let session =
            run_shell_session(&rom, &lowered, &cfg, &prompt_ids, 0xBEEF).expect("session runs");
        assert!(
            session.all_gates_pass(),
            "shell session gates: {:?}",
            session.bg_mismatches
        );
        let host = host_prompt_sample_generate(&lowered, &cfg, &prompt_ids, 0xBEEF, 12);
        assert_eq!(host.len(), 12);
        assert!(session.rom_sequence.len() <= host.len());
        assert_eq!(
            session.rom_sequence.as_slice(),
            &host[..session.rom_sequence.len()],
            "shell-verified prefix must match the host stream"
        );
    }

    /// Zero-fraction accounting covers every ternary matrix exactly once.
    #[test]
    fn ternary_zero_fraction_counts_every_matrix() {
        let ck = synthetic_state_checkpoint(7);
        let t = ck.topology();
        let (total, frac) = ternary_zero_fraction(&ck);
        let expected = (t.state_slots * t.d_model
            + t.d_model * t.state_slots
            + t.n_blocks * 2 * (t.d_ff * t.d_model)) as u64;
        assert_eq!(total, expected);
        assert!(frac > 0.0 && frac < 1.0);
    }
}
