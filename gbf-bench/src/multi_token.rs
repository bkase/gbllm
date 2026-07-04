//! Multi-token sustained generation gate (bd-2gc6p): >= 256 consecutive
//! tokens generated entirely on-device from the real trained checkpoint,
//! with continued byte-exact agreement against the host integer evaluator.
//!
//! The ROM ([`build_multi_token_rom`]) loops the one-token forward pass on
//! the Game Boy itself: forward pass -> argmax -> write the byte to an
//! output ring in WRAM -> feed it back as the next input. The host only
//! pokes the seed byte, then observes the run through passive PC traps at
//! the per-token boundary. Gates:
//!
//! 1. **Sequence agreement**: the WRAM output ring must equal the host
//!    integer evaluator's generation byte-for-byte, for every seed.
//! 2. **Checkpoint agreement**: the per-token WRAM dumps (block-0
//!    norm/up-acc/gelu/down-acc, 4 residuals, final norm, 256 i24 logits,
//!    argmax) must be byte-exact at the FIRST and LAST token boundary, so a
//!    divergence is localizable to a stage.
//! 3. **Sustained-run health**: SP back at its home value at every token
//!    boundary; declared-untouched WRAM regions unchanged across the whole
//!    run; per-token cycle counts stable (flagged if max/min > 1.05).

use std::path::Path;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TraceDropPolicy,
};
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_model::{
    DONE_ADDR, INPUT_ADDR, MODEL_STACK_TOP, MultiTokenRom, OUT_BASE, build_multi_token_rom,
};
use gbf_kernel::model_ref::{IntForwardTrace, IntLoweredModel, N_BLOCKS};
use serde::Serialize;

use crate::one_token::{
    CheckpointBundle, DMG_M_CYCLES_PER_SECOND, OneTokenError, SegmentMismatch, expected_segments,
    load_checkpoint,
};

/// The gate's generation length (the bead requires >= 256 consecutive steps).
pub const GENERATION_TOKENS: u16 = 256;

/// Seed bytes for the sequence-equality gate (>= 4 required): newline,
/// space, 'T', 'e'.
pub const GENERATION_SEEDS: [u8; 4] = [0x0A, 0x20, 0x54, 0x65];

/// WRAM regions the token loop must never write, as `[start, end)` ranges.
/// Everything outside these is a declared arena (activations, accumulators,
/// residual, scratch, logits, LUT pages, dumps, control bytes, output ring,
/// stack). Snapshotted before the run and re-read after the last token.
pub const UNTOUCHED_WRAM_REGIONS: &[(u16, u16)] = &[
    (0xC2E0, 0xC300), // between scratch and logits
    (0xC600, 0xC700), // between logits and head LUT pages
    (0xCC60, 0xCC80), // between control bytes and block-0 norm dump
    (0xCCC0, 0xCD00), // between norm dump and up-acc dump
    (0xCE80, 0xCF00), // between gelu dump and down-acc dump
    (0xCF80, 0xD000), // between down-acc dump and the output ring
    (0xD100, 0xDFC0), // between the output ring and the stack arena
    (0xDFF0, 0xE000), // above the stack home (push writes strictly below)
];

// ---------------------------------------------------------------------------
// host mirror
// ---------------------------------------------------------------------------

/// Host-side generation: the canonical integer evaluator run in the same
/// argmax-feedback loop the ROM executes.
pub struct HostGeneration {
    pub sequence: Vec<u8>,
    pub first_trace: IntForwardTrace,
    pub last_trace: IntForwardTrace,
}

/// Generate `n_tokens` bytes on the host by feeding each argmax back as the
/// next input, keeping the first and last full traces for dump comparison.
#[must_use]
pub fn host_generate(lowered: &IntLoweredModel, seed: u8, n_tokens: u16) -> HostGeneration {
    assert!(n_tokens >= 1, "host generation needs at least one token");
    let mut input = seed;
    let mut sequence = Vec::with_capacity(usize::from(n_tokens));
    let mut first_trace = None;
    let mut last_trace = None;
    for t in 0..n_tokens {
        let trace = lowered.forward(input);
        input = trace.argmax;
        sequence.push(trace.argmax);
        if t == 0 {
            first_trace = Some(trace.clone());
        }
        if t == n_tokens - 1 {
            last_trace = Some(trace);
        }
    }
    HostGeneration {
        sequence,
        first_trace: first_trace.expect("n_tokens >= 1"),
        last_trace: last_trace.expect("n_tokens >= 1"),
    }
}

// ---------------------------------------------------------------------------
// per-seed emulator run
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CycleStats {
    pub min: u64,
    pub median: u64,
    pub max: u64,
    pub mean: u64,
    pub max_over_min: f64,
    /// The bigram forward pass is data-dependent only in branch timing, so
    /// per-token cycles must be nearly constant; flagged if max/min > 1.05.
    pub stable_within_5pct: bool,
}

impl CycleStats {
    pub(crate) fn from_samples(samples: &[u64]) -> Self {
        assert!(!samples.is_empty(), "cycle stats need at least one token");
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let min = sorted[0];
        let max = *sorted.last().expect("non-empty");
        let ratio = max as f64 / min.max(1) as f64;
        Self {
            min,
            median: sorted[sorted.len() / 2],
            max,
            mean: samples.iter().sum::<u64>() / samples.len() as u64,
            max_over_min: ratio,
            stable_within_5pct: ratio <= 1.05,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WramViolation {
    pub region_start: u16,
    pub region_end: u16,
    pub first_bad_addr: u16,
    pub before: u8,
    pub after: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeedRun {
    pub seed: u8,
    pub n_tokens: u16,
    pub host_sequence_sha256: String,
    pub rom_sequence_sha256: String,
    /// Primary gate: the WRAM output ring equals the host generation.
    pub sequences_match: bool,
    pub first_divergence_index: Option<usize>,
    /// All WRAM checkpoint dumps byte-exact at the first token boundary.
    pub first_token_checkpoints_byte_exact: bool,
    /// All WRAM checkpoint dumps byte-exact at the last token boundary.
    pub last_token_checkpoints_byte_exact: bool,
    pub checkpoint_mismatches: Vec<SegmentMismatch>,
    pub cycles: CycleStats,
    /// SP observed equal to `MODEL_STACK_TOP` at every token boundary.
    pub sp_home_every_token: bool,
    pub sp_violation_tokens: Vec<u16>,
    /// Declared-untouched WRAM regions identical before and after the run.
    pub wram_untouched_regions_ok: bool,
    pub wram_violations: Vec<WramViolation>,
    pub done_flag_set: bool,
    /// Committed sample-text file name for this seed's generated bytes.
    pub sample_file: String,
    /// The raw generated bytes (written to the sample file by the runner,
    /// not embedded in the JSON report).
    #[serde(skip)]
    pub rom_sequence: Vec<u8>,
}

impl SeedRun {
    /// Every gate and health check for this seed.
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

fn compare_dumps(
    emu: &Emulator,
    trace: &IntForwardTrace,
    token: u16,
    mismatches: &mut Vec<SegmentMismatch>,
) -> Result<bool, OneTokenError> {
    let mut all_ok = true;
    for (name, addr, expected) in expected_segments(trace) {
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

/// Execute the multi-token ROM for one seed and run every gate against the
/// host mirror. The generation loop runs entirely on-device; the host only
/// pokes the seed and observes via passive PC traps at the token boundary.
pub fn run_seed_generation(
    rom: &MultiTokenRom,
    lowered: &IntLoweredModel,
    seed: u8,
) -> Result<SeedRun, OneTokenError> {
    let host = host_generate(lowered, seed, rom.n_tokens);

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    emu.poke(INPUT_ADDR, seed)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;

    // Snapshot the declared-untouched regions before executing anything.
    let baseline: Vec<Vec<u8>> = UNTOUCHED_WRAM_REGIONS
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
            // Step off the boundary PC through the loop-back jump; the trap
            // check runs before execution, so the same PC cannot re-trap
            // without an intermediate target.
            run_to(&mut emu, rom.token_start_pc, "loop head")?;
        }
        run_to(&mut emu, rom.token_boundary_pc, "token boundary")?;
        let now = emu.m_cycle_count_floor().0;
        per_token_cycles.push(now.saturating_sub(prev_cycles));
        prev_cycles = now;

        if emu.regs().sp != MODEL_STACK_TOP {
            sp_violation_tokens.push(t);
        }
        if t == 0 {
            first_token_ok = compare_dumps(&emu, &host.first_trace, t, &mut checkpoint_mismatches)?;
        }
        if t == rom.n_tokens - 1 {
            last_token_ok = compare_dumps(&emu, &host.last_trace, t, &mut checkpoint_mismatches)?;
        }
    }

    run_to(&mut emu, rom.token_end_pc, "token end")?;
    let done_flag_set = emu
        .peek(DONE_ADDR)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?
        == 1;

    // Primary artifact: the generated byte sequence in the output ring.
    let rom_sequence = emu
        .peek_range(OUT_BASE, usize::from(rom.n_tokens))
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    let first_divergence_index = host
        .sequence
        .iter()
        .zip(rom_sequence.iter())
        .position(|(h, r)| h != r);
    let sequences_match =
        first_divergence_index.is_none() && host.sequence.len() == rom_sequence.len();

    // Whole-run memory stability: untouched regions must be unchanged.
    let mut wram_violations = Vec::new();
    for (&(start, end), before) in UNTOUCHED_WRAM_REGIONS.iter().zip(baseline.iter()) {
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

    Ok(SeedRun {
        seed,
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
        sample_file: format!("sample_seed_0x{seed:02X}.txt"),
        rom_sequence,
    })
}

// ---------------------------------------------------------------------------
// evidence report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MultiCheckpointFacts {
    pub export_dir: String,
    pub manifest_schema: String,
    /// sha256 of the manifest file itself; the manifest pins every tensor's
    /// sha256, all of which are verified on load.
    pub manifest_sha256: String,
    pub trainer_git_sha: String,
    pub tensors_verified_sha256: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiRomFacts {
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
pub struct MultiTokenReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub git_sha: String,
    pub checkpoint: MultiCheckpointFacts,
    pub rom: MultiRomFacts,
    pub seeds: Vec<u8>,
    /// Sequence gate over all seeds (byte-exact ring vs host generation).
    pub all_sequences_match: bool,
    /// Every health check (SP home, WRAM stability, cycle stability, dump
    /// agreement at first/last token, done flag) over all seeds.
    pub all_health_checks_pass: bool,
    pub mean_m_cycles_per_token: u64,
    pub seconds_per_token_dmg: f64,
    pub runs: Vec<SeedRun>,
    pub caveats: Vec<String>,
}

/// Build the multi-token ROM from the committed checkpoint export and run
/// the full gate over `seeds`.
pub fn run_multi_token_generation(
    repo_root: &Path,
    export_dir_rel: &str,
    seeds: &[u8],
    n_tokens: u16,
) -> Result<MultiTokenReport, OneTokenError> {
    let export_dir = repo_root.join(export_dir_rel);
    let bundle: CheckpointBundle = load_checkpoint(&export_dir)?;
    let manifest_bytes =
        std::fs::read(export_dir.join("manifest.json")).map_err(|e| OneTokenError::Io {
            path: export_dir.join("manifest.json"),
            reason: e.to_string(),
        })?;
    let lowered = IntLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    let rom =
        build_multi_token_rom(&lowered, n_tokens).map_err(|e| OneTokenError::Rom(e.to_string()))?;

    let mut runs = Vec::new();
    for &seed in seeds {
        runs.push(run_seed_generation(&rom, &lowered, seed)?);
    }
    let all_sequences_match = runs.iter().all(|r| r.sequences_match);
    let all_health_checks_pass = runs.iter().all(SeedRun::all_checks_pass);
    let mean_m_cycles_per_token = runs.iter().map(|r| r.cycles.mean).sum::<u64>()
        / u64::try_from(runs.len().max(1)).expect("run count fits u64");

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(MultiTokenReport {
        schema: "multi_token_generation.v1",
        bead: "bd-2gc6p",
        git_sha,
        checkpoint: MultiCheckpointFacts {
            export_dir: export_dir_rel.to_string(),
            manifest_schema: bundle.manifest_schema,
            manifest_sha256: sha256(&manifest_bytes).to_hex(),
            trainer_git_sha: bundle.manifest_git_sha,
            tensors_verified_sha256: bundle.tensors_verified,
        },
        rom: MultiRomFacts {
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
        seeds: seeds.to_vec(),
        all_sequences_match,
        all_health_checks_pass,
        mean_m_cycles_per_token,
        seconds_per_token_dmg: mean_m_cycles_per_token as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        runs,
        caveats: vec![
            format!(
                "Bigram-context model: each step depends only on the previous byte, so \
                 deterministic argmax generation enters a repeating cycle as soon as any byte \
                 recurs; the sample text is expected to be babble with local structure, not \
                 coherent prose. All {N_BLOCKS} blocks still execute in full every token."
            ),
            "The host observes the run through passive PC traps at the per-token boundary; \
             the generation loop (forward pass, argmax, ring write, feedback) executes \
             entirely in ROM code on the emulated CPU with no host pokes after the seed."
                .to_string(),
            "Token 0's cycle count is measured from the loop head and later tokens from \
             boundary to boundary, so token 0 excludes one loop-back jump (4 M-cycles); \
             this is far below the 5% stability threshold."
                .to_string(),
        ],
    })
}

/// Render the generated bytes as committed sample text: printable ASCII and
/// newlines pass through, everything else becomes an escaped `\xNN`.
#[must_use]
pub fn render_sample_text(sequence: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(sequence.len() + 16);
    for &b in sequence {
        match b {
            0x20..=0x7E => out.push(b as char),
            0x0A => out.push('\n'),
            _ => {
                let _ = write!(out, "\\x{b:02X}");
            }
        }
    }
    out
}

/// Render the report README (generated, not hand-written).
#[must_use]
pub fn multi_report_to_markdown(report: &MultiTokenReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Multi-token sustained generation ({})",
        report.schema
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "First sustained on-device text generation from the real trained dense-ternary \
         checkpoint (bd-2gc6p): {} consecutive tokens per seed, generated entirely by the \
         emulated Game Boy (forward pass -> argmax -> WRAM output ring -> fed back as the \
         next input), with byte-exact agreement against the host integer evaluator. \
         Generated by `cargo run -p gbf-bench --bin multi-token`; every number below is \
         program output at git `{}`.",
        report.rom.n_tokens, report.git_sha
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
    let _ = writeln!(out, "## ROM");
    let _ = writeln!(out);
    let r = &report.rom;
    let _ = writeln!(
        out,
        "- {} bytes ({} banks), driver {} B, weight code {} B in {} chunks, tables {} B; \
         generation loop of {} tokens in ROM (token boundary trap at {:#06x})",
        r.rom_bytes,
        r.bank_count,
        r.driver_bytes,
        r.weight_code_bytes,
        r.weight_chunk_count,
        r.table_bytes,
        r.n_tokens,
        r.token_boundary_pc
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Gate — sequence + checkpoint agreement");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- **Sequences: {}** — {}/{} seeds produced a {}-byte on-device sequence identical \
         to the host integer evaluator",
        if report.all_sequences_match {
            "PASS"
        } else {
            "FAIL"
        },
        report.runs.iter().filter(|r| r.sequences_match).count(),
        report.runs.len(),
        report.rom.n_tokens
    );
    let _ = writeln!(
        out,
        "- **Health: {}** — SP home at every token boundary, untouched WRAM regions \
         unchanged across the run, per-token cycles stable, first/last-token WRAM \
         checkpoint dumps byte-exact, done flag set",
        if report.all_health_checks_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| seed | sequence match | first/last dumps | cycles min | median | max | max/min | SP home | WRAM clean | sample |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|");
    for run in &report.runs {
        let _ = writeln!(
            out,
            "| 0x{:02X} | {} | {}/{} | {} | {} | {} | {:.5} | {} | {} | `{}` |",
            run.seed,
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
        "- Mean over all seeds and tokens: **{} M-cycles/token** = {:.3} s/token on DMG",
        report.mean_m_cycles_per_token, report.seconds_per_token_dmg
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Sample text");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "The `sample_seed_0x*.txt` files are the project's first on-device-generated text: \
         raw generated bytes rendered as printable ASCII (newlines kept, other bytes escaped \
         as `\\xNN`). This is a bigram-context model — expect repetitive babble with local \
         structure, not coherent prose."
    );
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
    use gbf_kernel::model_ref::synthetic_checkpoint;

    /// Full-stack smoke on a synthetic checkpoint: the on-device generation
    /// loop must reproduce the host feedback loop byte-exactly and stay
    /// healthy (SP home, WRAM stable, dumps exact at first/last token).
    /// This is the same machinery the real-checkpoint gate uses, with a
    /// short run to keep the unoptimized-emulator cost bounded.
    #[test]
    fn multi_token_rom_matches_host_generation_on_synthetic_model() {
        let ck = synthetic_checkpoint(21);
        let lowered = IntLoweredModel::lower(&ck).expect("lowers");
        let rom = build_multi_token_rom(&lowered, 6).expect("builds");
        for seed in [0x41u8, 0x0A] {
            let run = run_seed_generation(&rom, &lowered, seed).expect("runs");
            assert!(
                run.sequences_match,
                "seed 0x{seed:02X}: ROM sequence diverged at {:?}",
                run.first_divergence_index
            );
            assert!(
                run.first_token_checkpoints_byte_exact && run.last_token_checkpoints_byte_exact,
                "seed 0x{seed:02X}: dump mismatches {:?}",
                run.checkpoint_mismatches
            );
            assert!(
                run.sp_home_every_token,
                "seed 0x{seed:02X}: SP violations at tokens {:?}",
                run.sp_violation_tokens
            );
            assert!(
                run.wram_untouched_regions_ok,
                "seed 0x{seed:02X}: WRAM violations {:?}",
                run.wram_violations
            );
            assert!(run.done_flag_set, "seed 0x{seed:02X}: done flag not set");
            assert_eq!(run.rom_sequence.len(), 6);
            assert!(run.cycles.min > 0);
        }
    }

    #[test]
    fn render_sample_text_escapes_non_printable_bytes() {
        assert_eq!(
            render_sample_text(&[0x54, 0x68, 0x65, 0x0A, 0x00, 0xFF, 0x20]),
            "The\n\\x00\\xFF "
        );
    }
}
