//! Latency budget gate (bd-3l3tl): measured sustained-generation latency of
//! the deployed d192 shell ROM over a real scripted joypad session, asserted
//! against the 30 s/char UX budget (bd-3771m, bkase budget revision 2,
//! 2026-07-04; planv0 2026-07-04 amendment).
//!
//! The gate row is the REAL d192 distilled-student shell ROM (the deployed
//! demo artifact, `d192_real_bringup.v2` substrate); the arm-B v0 shell ROM
//! is included as a reference row only. All timing is DMG M-cycle-accurate
//! emulated time from the same scripted session machinery the interactive
//! shell and d192-real packets use; one generated token renders exactly one
//! charset_v1 char, so seconds/token == seconds/char.
//!
//! Evidence (`latency_gate.v1`) is produced by the `latency-gate` bin —
//! never hand-written.

use std::path::Path;

use gbf_emu::DMG_FRAME_CLOCK_CYCLES;
use gbf_kernel::asm_impl_shell::build_state_shell_rom;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::IntStateLoweredModel;
use serde::Serialize;

use crate::d192_real::D192_REAL_EXPORT_DIR;
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};
use crate::sampling::SamplerSettingFacts;
use crate::shell::{
    SHELL_TEMPERATURE, SHELL_TOP_K, char_to_id, run_shell_session_observed, shell_font_tiles,
};
use crate::stateful::{STATE_EXPORT_DIR, StateCheckpointFacts, load_state_checkpoint};

/// The UX budget the gate asserts (seconds per generated char on DMG).
pub const LATENCY_BUDGET_S_PER_CHAR: f64 = 30.0;

/// Minimum consecutive on-device tokens the gate must measure.
pub const LATENCY_MIN_TOKENS: usize = 128;

/// The scripted prompt (20 chars, exactly the shell prompt cap) and RNG
/// seed — the same setting as the committed d192-real demo sample.
pub const LATENCY_GATE_PROMPT: &str = "The machines dreamed";
pub const LATENCY_GATE_RNG_SEED: u16 = 0x5EED;

/// DMG frame period in M-cycles (70224 clock cycles / 4).
pub const DMG_FRAME_M_CYCLES: u64 = DMG_FRAME_CLOCK_CYCLES.0 / 4;

/// Absolute tolerance on the measured idle joypad-poll rate vs the DMG
/// frame rate (Hz).
pub const IDLE_POLL_HZ_TOLERANCE: f64 = 0.1;

/// Expected idle joypad-poll rate: one poll per PPU frame (59.7275 Hz).
#[must_use]
pub fn dmg_frame_hz() -> f64 {
    DMG_M_CYCLES_PER_SECOND as f64 / DMG_FRAME_M_CYCLES as f64
}

/// Nearest-rank percentile over raw M-cycle deltas (`p` in (0, 100]).
/// Deterministic pure-integer selection; panics on an empty slice.
#[must_use]
pub fn percentile_nearest_rank(samples: &[u64], p: f64) -> u64 {
    assert!(!samples.is_empty(), "percentile of empty sample set");
    assert!(p > 0.0 && p <= 100.0, "percentile out of range");
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.max(1) - 1]
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenLatencyStats {
    pub n_tokens: usize,
    pub min_m_cycles: u64,
    pub p50_m_cycles: u64,
    pub p95_m_cycles: u64,
    pub max_m_cycles: u64,
    pub mean_m_cycles: u64,
    pub p50_s_per_char: f64,
    pub p95_s_per_char: f64,
    pub max_s_per_char: f64,
    pub mean_s_per_char: f64,
    /// The first token boundary samples from the warmup forward's logits
    /// (sample + render only, no forward pass), so its delta is structurally
    /// small; it is included in the percentiles above and reported here.
    pub first_boundary_m_cycles: u64,
}

impl TokenLatencyStats {
    /// Compute from the session's consecutive token-boundary M-cycle deltas.
    #[must_use]
    pub fn from_deltas(deltas: &[u64]) -> Self {
        assert!(!deltas.is_empty(), "no token boundaries measured");
        let s = |c: u64| c as f64 / DMG_M_CYCLES_PER_SECOND as f64;
        let min = *deltas.iter().min().expect("nonempty");
        let max = *deltas.iter().max().expect("nonempty");
        let p50 = percentile_nearest_rank(deltas, 50.0);
        let p95 = percentile_nearest_rank(deltas, 95.0);
        let mean = deltas.iter().sum::<u64>() / deltas.len() as u64;
        Self {
            n_tokens: deltas.len(),
            min_m_cycles: min,
            p50_m_cycles: p50,
            p95_m_cycles: p95,
            max_m_cycles: max,
            mean_m_cycles: mean,
            p50_s_per_char: s(p50),
            p95_s_per_char: s(p95),
            max_s_per_char: s(max),
            mean_s_per_char: s(mean),
            first_boundary_m_cycles: deltas[0],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IdlePollStats {
    /// Frames measured (one idle-loop iteration per injected typing frame).
    pub n_frames: usize,
    pub min_m_cycles_per_frame: u64,
    pub max_m_cycles_per_frame: u64,
    pub mean_m_cycles_per_frame: f64,
    pub measured_hz: f64,
    pub expected_hz: f64,
    pub tolerance_hz: f64,
    pub within_tolerance: bool,
}

impl IdlePollStats {
    /// Compute from the session's idle-frame M-cycle deltas (keyboard
    /// phase: the typing script drives one idle iteration per PPU frame).
    #[must_use]
    pub fn from_deltas(deltas: &[u64]) -> Self {
        assert!(!deltas.is_empty(), "no idle frames measured");
        #[allow(clippy::cast_precision_loss)]
        let mean = deltas.iter().sum::<u64>() as f64 / deltas.len() as f64;
        let hz = DMG_M_CYCLES_PER_SECOND as f64 / mean;
        let expected = dmg_frame_hz();
        Self {
            n_frames: deltas.len(),
            min_m_cycles_per_frame: *deltas.iter().min().expect("nonempty"),
            max_m_cycles_per_frame: *deltas.iter().max().expect("nonempty"),
            mean_m_cycles_per_frame: mean,
            measured_hz: hz,
            expected_hz: expected,
            tolerance_hz: IDLE_POLL_HZ_TOLERANCE,
            within_tolerance: (hz - expected).abs() <= IDLE_POLL_HZ_TOLERANCE,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyRow {
    /// "gate" (asserted) or "reference" (recorded, not asserted).
    pub role: String,
    pub model: String,
    pub checkpoint: StateCheckpointFacts,
    pub macs_per_token: u64,
    pub sampler: SamplerSettingFacts,
    pub prompt: String,
    pub rng_seed: u16,
    /// The scripted session's own correctness gates (boot chrome, prompt
    /// echo, host byte-identity, transcript BG, return to idle).
    pub session_gates_pass: bool,
    pub sequences_match_host: bool,
    pub n_tokens_generated: usize,
    pub min_required_tokens: usize,
    pub enough_tokens: bool,
    pub warmup_mean_m_cycles_per_char: u64,
    pub warmup_s_per_prompt_char: f64,
    pub tokens: TokenLatencyStats,
    pub idle_poll: IdlePollStats,
    pub budget_s_per_char: f64,
    pub p50_within_budget: bool,
    pub p95_within_budget: bool,
    pub max_within_budget: bool,
    /// Everything above, conjoined (what the gate asserts for the gate row).
    pub row_pass: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyGateReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub upstream_beads: Vec<&'static str>,
    pub git_sha: String,
    pub budget_s_per_char: f64,
    pub budget_provenance: &'static str,
    pub token_char_equivalence: &'static str,
    /// Row 0 is the deployed d192 gate row; row 1 the arm-B reference row.
    pub rows: Vec<LatencyRow>,
    /// True iff the gate row passes (reference rows are never asserted).
    pub gate_pass: bool,
    pub quick_mode: bool,
    pub caveats: Vec<String>,
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

#[allow(clippy::too_many_lines)]
fn measure_row(
    repo_root: &Path,
    role: &str,
    model: &str,
    export_dir_rel: &str,
    n_gen_tokens: u8,
    min_required_tokens: usize,
) -> Result<LatencyRow, OneTokenError> {
    let bundle = load_state_checkpoint(&repo_root.join(export_dir_rel))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(SHELL_TOP_K, step, SHELL_TEMPERATURE)
        .map_err(|e| OneTokenError::Model(format!("sampler config: {e}")))?;
    let font = shell_font_tiles();
    let rom = build_state_shell_rom(&lowered, &cfg, n_gen_tokens, &font)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let prompt_ids: Vec<u8> = LATENCY_GATE_PROMPT
        .chars()
        .map(|c| char_to_id(c).expect("gate prompt chars are charset_v1 printables"))
        .collect();
    let session = run_shell_session_observed(
        &rom,
        &lowered,
        &cfg,
        &prompt_ids,
        LATENCY_GATE_RNG_SEED,
        false,
    )?;

    let tokens = TokenLatencyStats::from_deltas(&session.token_boundary_m_cycles);
    let idle_poll = IdlePollStats::from_deltas(&session.idle_frame_m_cycles);
    let warm_mean = session.warm_boundary_m_cycles.iter().sum::<u64>()
        / session.warm_boundary_m_cycles.len().max(1) as u64;
    let enough_tokens = session.n_tokens_generated >= min_required_tokens;
    let p50_ok = tokens.p50_s_per_char <= LATENCY_BUDGET_S_PER_CHAR;
    let p95_ok = tokens.p95_s_per_char <= LATENCY_BUDGET_S_PER_CHAR;
    let max_ok = tokens.max_s_per_char <= LATENCY_BUDGET_S_PER_CHAR;
    let row_pass = session.all_gates_pass()
        && session.sequences_match
        && enough_tokens
        && p50_ok
        && p95_ok
        && max_ok
        && idle_poll.within_tolerance;

    Ok(LatencyRow {
        role: role.to_string(),
        model: model.to_string(),
        checkpoint: StateCheckpointFacts {
            export_dir: export_dir_rel.to_string(),
            manifest_schema: bundle.manifest_schema,
            manifest_sha256: bundle.manifest_sha256,
            trainer_git_sha: bundle.manifest_git_sha,
            tensors_verified_sha256: bundle.tensors_verified,
        },
        macs_per_token: bundle.topology.macs_per_token(),
        sampler: SamplerSettingFacts {
            top_k: cfg.k(),
            scale_q16: cfg.scale_q16(),
            requested_temperature: SHELL_TEMPERATURE,
            effective_temperature: cfg.effective_temperature(step),
        },
        prompt: LATENCY_GATE_PROMPT.to_string(),
        rng_seed: LATENCY_GATE_RNG_SEED,
        session_gates_pass: session.all_gates_pass(),
        sequences_match_host: session.sequences_match,
        n_tokens_generated: session.n_tokens_generated,
        min_required_tokens,
        enough_tokens,
        warmup_mean_m_cycles_per_char: warm_mean,
        warmup_s_per_prompt_char: warm_mean as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        tokens,
        idle_poll,
        budget_s_per_char: LATENCY_BUDGET_S_PER_CHAR,
        p50_within_budget: p50_ok,
        p95_within_budget: p95_ok,
        max_within_budget: max_ok,
        row_pass,
    })
}

/// Run the latency gate: the deployed d192 shell ROM as the asserted gate
/// row, the arm-B v0 shell ROM as a reference row.
pub fn run_latency_gate(repo_root: &Path, quick: bool) -> Result<LatencyGateReport, OneTokenError> {
    let (n_gen, min_tokens) = if quick {
        (8u8, 1usize)
    } else {
        (200u8, LATENCY_MIN_TOKENS)
    };
    let gate_row = measure_row(
        repo_root,
        "gate",
        "d192/ff384/6blk/slots192 distilled student (deployed demo model)",
        D192_REAL_EXPORT_DIR,
        n_gen,
        min_tokens,
    )?;
    let reference_row = measure_row(
        repo_root,
        "reference",
        "d64/ff128/4blk/slots64 arm-B v0 bring-up checkpoint",
        STATE_EXPORT_DIR,
        n_gen,
        min_tokens,
    )?;
    let gate_pass = gate_row.row_pass;
    let mut caveats = vec![
        "All timing is DMG M-cycle-accurate emulated time (gbf-emu headless), interrupts \
         disabled, SP repurposed inside weight chunks; production kernels pay yield/safe-point \
         overhead on top of these numbers."
            .to_string(),
        "Token-boundary deltas include the VBlank-aligned transcript render, i.e. they are the \
         real UI cadence a player sees, not bare forward-pass time."
            .to_string(),
        "The first token boundary samples from the prompt-warmup logits (no forward pass), so \
         its delta is structurally small; it is included in the percentile set and reported \
         separately per row."
            .to_string(),
        "The reference row (arm-B) is recorded for context only and is not asserted by this \
         gate."
            .to_string(),
        "The original bead title targeted <= 10 s/char at 70% CPU; the governing budget was \
         revised to 30 s/char (bd-3771m comment 2026-07-04 15:15 UTC, quality-first). This \
         gate asserts the revised budget against 100%-CPU sustained measurements (harder than \
         the 70%-CPU formulation: no reserve is subtracted from the measured numbers)."
            .to_string(),
    ];
    if quick {
        caveats
            .push("QUICK MODE: development smoke sizes; this report is not evidence.".to_string());
    }
    Ok(LatencyGateReport {
        schema: "latency_gate.v1",
        bead: "bd-3l3tl",
        upstream_beads: vec!["bd-2gc6p", "bd-1kbv1", "bd-pp43d", "bd-3771m"],
        git_sha: git_head(repo_root),
        budget_s_per_char: LATENCY_BUDGET_S_PER_CHAR,
        budget_provenance: "bd-3771m comment 2026-07-04 15:15 UTC (bkase budget revision 2, \
                            planv0 2026-07-04 quality-first amendment): up to 30 s/char is \
                            acceptable; optimize for model quality",
        token_char_equivalence: "charset_v1 is a character vocabulary: one generated token \
                                 renders exactly one char, so seconds/token == seconds/char",
        rows: vec![gate_row, reference_row],
        gate_pass,
        quick_mode: quick,
        caveats,
    })
}

/// Render the packet README (generated, never hand-written).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn latency_report_to_markdown(r: &LatencyGateReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Latency budget gate ({})", r.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Sustained generation latency of the deployed d192 shell ROM, measured over a real \
         scripted joypad session (type `{}` on the on-screen keyboard, START, generate) and \
         asserted against the **{} s/char** UX budget. Generated by `cargo run --release -p \
         gbf-bench --bin latency-gate`; every number is program output at git `{}`.",
        r.rows[0].prompt, r.budget_s_per_char, r.git_sha
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "- Bead: {} (budget: {})", r.bead, r.budget_provenance);
    let _ = writeln!(out, "- {}", r.token_char_equivalence);
    let _ = writeln!(
        out,
        "- Gate verdict: **{}**",
        if r.gate_pass { "PASS" } else { "FAIL" }
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Rows");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| role | model | tokens | p50 s/char | p95 s/char | max s/char | mean s/char | \
         budget | within budget (p50/p95/max) | session gates | host byte-identity |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|");
    for row in &r.rows {
        let t = &row.tokens;
        let _ = writeln!(
            out,
            "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {}/{}/{} | {} | {} |",
            row.role,
            row.model,
            t.n_tokens,
            t.p50_s_per_char,
            t.p95_s_per_char,
            t.max_s_per_char,
            t.mean_s_per_char,
            row.budget_s_per_char,
            yn(row.p50_within_budget),
            yn(row.p95_within_budget),
            yn(row.max_within_budget),
            yn(row.session_gates_pass),
            yn(row.sequences_match_host),
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Detail per row");
    for row in &r.rows {
        let t = &row.tokens;
        let ip = &row.idle_poll;
        let _ = writeln!(out);
        let _ = writeln!(out, "### {} — {}", row.role, row.model);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- Checkpoint `{}` (manifest sha256 `{}`), {} MACs/token, top-k {} at T {} \
             (effective {:.4}), prompt `{}`, RNG seed 0x{:04X}",
            row.checkpoint.export_dir,
            &row.checkpoint.manifest_sha256[..16],
            row.macs_per_token,
            row.sampler.top_k,
            row.sampler.requested_temperature,
            row.sampler.effective_temperature,
            row.prompt,
            row.rng_seed
        );
        let _ = writeln!(
            out,
            "- Tokens: {} consecutive on-device (required >= {}): min {} / p50 {} / p95 {} / \
             max {} M-cycles; first boundary {} M-cycles (samples from warmup logits, no \
             forward pass)",
            t.n_tokens,
            row.min_required_tokens,
            t.min_m_cycles,
            t.p50_m_cycles,
            t.p95_m_cycles,
            t.max_m_cycles,
            t.first_boundary_m_cycles
        );
        let _ = writeln!(
            out,
            "- Warmup: {} M-cycles mean per prompt char = {:.3} s/char",
            row.warmup_mean_m_cycles_per_char, row.warmup_s_per_prompt_char
        );
        let _ = writeln!(
            out,
            "- Keyboard-phase input poll: {} frames, {:.1} M-cycles/frame mean (min {} / max \
             {}) = **{:.4} Hz** vs expected {:.4} Hz (tolerance {} Hz): {}",
            ip.n_frames,
            ip.mean_m_cycles_per_frame,
            ip.min_m_cycles_per_frame,
            ip.max_m_cycles_per_frame,
            ip.measured_hz,
            ip.expected_hz,
            ip.tolerance_hz,
            if ip.within_tolerance { "PASS" } else { "FAIL" }
        );
        let _ = writeln!(
            out,
            "- Row verdict: **{}**{}",
            if row.row_pass { "PASS" } else { "FAIL" },
            if row.role == "reference" {
                " (reference only, not asserted)"
            } else {
                ""
            }
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Caveats");
    let _ = writeln!(out);
    for c in &r.caveats {
        let _ = writeln!(out, "- {c}");
    }
    out
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_are_exact_on_small_sets() {
        let v = vec![10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(percentile_nearest_rank(&v, 50.0), 50);
        assert_eq!(percentile_nearest_rank(&v, 95.0), 100);
        assert_eq!(percentile_nearest_rank(&v, 100.0), 100);
        assert_eq!(percentile_nearest_rank(&v, 10.0), 10);
        assert_eq!(percentile_nearest_rank(&[7], 50.0), 7);
        // unsorted input is sorted internally
        assert_eq!(percentile_nearest_rank(&[3, 1, 2], 50.0), 2);
    }

    #[test]
    fn token_stats_convert_m_cycles_to_seconds() {
        let stats = TokenLatencyStats::from_deltas(&[DMG_M_CYCLES_PER_SECOND * 2; 4]);
        assert_eq!(stats.n_tokens, 4);
        assert!((stats.p50_s_per_char - 2.0).abs() < 1e-12);
        assert!((stats.max_s_per_char - 2.0).abs() < 1e-12);
        assert_eq!(stats.first_boundary_m_cycles, DMG_M_CYCLES_PER_SECOND * 2);
    }

    #[test]
    fn idle_poll_stats_accept_exact_frame_cadence_and_reject_double() {
        let ok = IdlePollStats::from_deltas(&[DMG_FRAME_M_CYCLES; 8]);
        assert!(ok.within_tolerance, "measured {} Hz", ok.measured_hz);
        assert!((ok.measured_hz - dmg_frame_hz()).abs() < 1e-9);
        let bad = IdlePollStats::from_deltas(&[DMG_FRAME_M_CYCLES * 2; 8]);
        assert!(!bad.within_tolerance);
    }

    #[test]
    fn dmg_frame_hz_matches_the_documented_rate() {
        assert!((dmg_frame_hz() - 59.7275).abs() < 1e-3);
        assert_eq!(DMG_FRAME_M_CYCLES, 17556);
    }
}
