//! Integer top-k/temperature sampling decode bring-up (bd-2mjkd): the
//! pinned exp-LUT + XorShift16 sampler (`gbf_kernel::decode`) running
//! against the stateful arm-B checkpoint, on host and on device, with the
//! same byte-exact agreement obligations as the argmax bring-up
//! (bd-x5l2s).
//!
//! Phases:
//! 1. Host sampler semantics live in `gbf_kernel::decode` (unit-tested
//!    there, including full-period RNG and distribution sanity).
//! 2. ROM gate: the sampling multi-token ROM must generate
//!    [`SAMPLING_GENERATION_TOKENS`] tokens per `(seed_byte, rng_seed)`
//!    combo entirely on-device, byte-identical to the host integer
//!    evaluator driving `decode::sample_topk` with the same seed, plus the
//!    usual SP/WRAM/cycle health checks.
//! 3. Cycle cost: the sampling epilogue's extra M-cycles/token over an
//!    argmax run measured in the same session.
//! 4. Evidence: `sampling_decode_bringup.v1` JSON + README + sample texts
//!    under `docs/experiments/sampling-decode/`, produced by the
//!    `sampling-decode` bin — never hand-written.

use std::path::Path;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, RunOutcome,
    TraceDropPolicy,
};
use gbf_foundation::sha256;
use gbf_kernel::asm_impl_state::{
    S_DONE_ADDR, S_INPUT_ADDR, S_RNG_ADDR, S_SAMPLED_ADDR, S_STACK_TOP, StateMultiTokenRom,
    StateWramLayout, build_state_multi_token_rom, build_state_multi_token_sampling_rom,
};
use gbf_kernel::decode::{
    EXP2_LUT_ALPHA, SamplerConfig, XORSHIFT16_SHIFTS, XorShift16, build_exp2_lut, sample_topk_trace,
};
use gbf_kernel::state_model_ref::{IntStateForwardTrace, IntStateLoweredModel};
use serde::Serialize;

use crate::multi_token::{CycleStats, WramViolation};
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError, SegmentMismatch};
use crate::stateful::{
    StateCheckpointFacts, id_to_char, load_state_checkpoint, render_char_sample,
    state_expected_segments,
};

/// On-device generation length per combo (the output ring caps at 256).
pub const SAMPLING_GENERATION_TOKENS: u16 = 256;

/// The `(seed_byte, rng_seed)` gate combos (>= 4 required): the four
/// argmax-gate seed chars crossed with distinct RNG seeds, including the
/// degenerate rng seed 0 (canonicalized to 1 on both sides).
pub const SAMPLING_GATE_COMBOS: [(u8, u16); 5] = [
    (19, 0xBEEF), // 'T'
    (26, 0x1234), // 'a'
    (62, 0xC0DE), // ' '
    (75, 0x0001), // '\n'
    (19, 0x0000), // 'T' with the canonicalized zero seed
];

// The untouched-WRAM regions are computed from the ROM's own
// `StateWramLayout` (`layout.untouched_regions()`), so the sampler tables
// and scratch are excluded automatically.

// ---------------------------------------------------------------------------
// host mirror
// ---------------------------------------------------------------------------

/// Host-side sampling generation mirror: zero state, XorShift16 seeded as
/// poked, one draw per token, sampled id fed back.
pub struct SamplingHostGeneration {
    pub sequence: Vec<u8>,
    pub first_trace: IntStateForwardTrace,
    pub last_trace: IntStateForwardTrace,
    pub first_pick: u8,
    pub last_pick: u8,
}

/// Generate `n_tokens` ids on the host with the pinned integer sampler.
#[must_use]
pub fn sampling_host_generate(
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    seed: u8,
    rng_seed: u16,
    n_tokens: u16,
) -> SamplingHostGeneration {
    assert!(n_tokens >= 1, "host generation needs at least one token");
    let mut rng = XorShift16::new(rng_seed);
    let mut state = lowered.zero_state();
    let mut input = seed;
    let mut sequence = Vec::with_capacity(usize::from(n_tokens));
    let mut first = None;
    let mut last = None;
    for t in 0..n_tokens {
        let trace = lowered.forward(input, &mut state);
        let pick = sample_topk_trace(&trace.logits, cfg, &mut rng).picked as u8;
        sequence.push(pick);
        input = pick;
        if t == 0 {
            first = Some((trace.clone(), pick));
        }
        if t == n_tokens - 1 {
            last = Some((trace, pick));
        }
    }
    let (first_trace, first_pick) = first.expect("n_tokens >= 1");
    let (last_trace, last_pick) = last.expect("n_tokens >= 1");
    SamplingHostGeneration {
        sequence,
        first_trace,
        last_trace,
        first_pick,
        last_pick,
    }
}

// ---------------------------------------------------------------------------
// ROM gate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SamplingComboRun {
    pub seed_id: u8,
    pub rng_seed: u16,
    pub n_tokens: u16,
    pub host_sequence_sha256: String,
    pub rom_sequence_sha256: String,
    pub sequences_match: bool,
    pub first_divergence_index: Option<usize>,
    pub first_token_checkpoints_byte_exact: bool,
    pub last_token_checkpoints_byte_exact: bool,
    pub sampled_id_matches_first_and_last: bool,
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

impl SamplingComboRun {
    #[must_use]
    pub fn all_checks_pass(&self) -> bool {
        self.sequences_match
            && self.first_token_checkpoints_byte_exact
            && self.last_token_checkpoints_byte_exact
            && self.sampled_id_matches_first_and_last
            && self.cycles.stable_within_5pct
            && self.sp_home_every_token
            && self.wram_untouched_regions_ok
            && self.done_flag_set
    }
}

fn compare_dumps(
    emu: &Emulator,
    trace: &IntStateForwardTrace,
    layout: &StateWramLayout,
    token: u16,
    mismatches: &mut Vec<SegmentMismatch>,
) -> Result<bool, OneTokenError> {
    let mut all_ok = true;
    for (name, addr, expected) in state_expected_segments(trace, layout) {
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

/// Execute the sampling multi-token ROM for one `(seed_byte, rng_seed)`
/// combo and gate it byte-exactly against the host mirror. The whole loop
/// (forward pass, argmax, top-k selection, LUT weights, RNG draw,
/// cumulative pick, ring write, feedback) executes on-device.
pub fn run_sampling_combo(
    rom: &StateMultiTokenRom,
    lowered: &IntStateLoweredModel,
    cfg: &SamplerConfig,
    seed: u8,
    rng_seed: u16,
) -> Result<SamplingComboRun, OneTokenError> {
    let host = sampling_host_generate(lowered, cfg, seed, rng_seed, rom.n_tokens);

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .trace_drop_policy(TraceDropPolicy::HaltAndError)
        .load_rom(&rom.rom)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    emu.poke(S_INPUT_ADDR, seed)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    let seed_bytes = rng_seed.to_le_bytes();
    emu.poke(S_RNG_ADDR, seed_bytes[0])
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    emu.poke(S_RNG_ADDR + 1, seed_bytes[1])
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;

    let untouched_regions = rom.layout.untouched_regions();
    let baseline: Vec<Vec<u8>> = untouched_regions
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
    let mut sampled_ok = true;

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
        if t == 0 || t == rom.n_tokens - 1 {
            let (trace, pick) = if t == 0 {
                (&host.first_trace, host.first_pick)
            } else {
                (&host.last_trace, host.last_pick)
            };
            let ok = compare_dumps(&emu, trace, &rom.layout, t, &mut checkpoint_mismatches)?;
            if t == 0 {
                first_token_ok = ok;
            } else {
                last_token_ok = ok;
            }
            let rom_pick = emu
                .peek(S_SAMPLED_ADDR)
                .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
            if rom_pick != pick {
                sampled_ok = false;
                checkpoint_mismatches.push(SegmentMismatch {
                    segment: format!("token{t}/sampled_id"),
                    wram_addr: S_SAMPLED_ADDR,
                    first_bad_offset: 0,
                    expected_byte: pick,
                    actual_byte: rom_pick,
                });
            }
        }
    }

    run_to(&mut emu, rom.token_end_pc, "token end")?;
    let done_flag_set = emu
        .peek(S_DONE_ADDR)
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?
        == 1;

    let rom_sequence = emu
        .peek_range(rom.layout.out, usize::from(rom.n_tokens))
        .map_err(|e| OneTokenError::Emulator(e.to_string()))?;
    let first_divergence_index = host
        .sequence
        .iter()
        .zip(rom_sequence.iter())
        .position(|(h, r)| h != r);
    let sequences_match =
        first_divergence_index.is_none() && host.sequence.len() == rom_sequence.len();

    let mut wram_violations = Vec::new();
    for (&(start, end), before) in untouched_regions.iter().zip(baseline.iter()) {
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

    Ok(SamplingComboRun {
        seed_id: seed,
        rng_seed,
        n_tokens: rom.n_tokens,
        host_sequence_sha256: sha256(&host.sequence).to_hex(),
        rom_sequence_sha256: sha256(&rom_sequence).to_hex(),
        sequences_match,
        first_divergence_index,
        first_token_checkpoints_byte_exact: first_token_ok,
        last_token_checkpoints_byte_exact: last_token_ok,
        sampled_id_matches_first_and_last: sampled_ok,
        checkpoint_mismatches,
        cycles: CycleStats::from_samples(&per_token_cycles),
        sp_home_every_token: sp_violation_tokens.is_empty(),
        sp_violation_tokens,
        wram_untouched_regions_ok: wram_violations.is_empty(),
        wram_violations,
        done_flag_set,
        sample_file: format!(
            "sample_rom_seed_{:02}_rng_{:04x}.txt",
            seed,
            if rng_seed == 0 { 1 } else { rng_seed }
        ),
        rom_sequence,
    })
}

// ---------------------------------------------------------------------------
// evidence report
// ---------------------------------------------------------------------------

/// The pinned design constants, restated by the program for the report.
#[derive(Debug, Clone, Serialize)]
pub struct SamplingDesignFacts {
    pub rng: String,
    pub rng_shifts: (u32, u32, u32),
    pub exp_lut: String,
    pub exp_lut_alpha: u32,
    pub exp_lut_sha256: String,
    pub lut_index_rule: String,
    pub threshold_rule: String,
    pub selection_rule: String,
    pub temperature_rule: String,
    pub logit_dequant_step: f64,
}

/// One sampler setting used for the gate or a qualitative sample.
#[derive(Debug, Clone, Serialize)]
pub struct SamplerSettingFacts {
    pub top_k: u8,
    pub scale_q16: u16,
    pub requested_temperature: f64,
    /// The temperature the u16-quantized scale actually realizes.
    pub effective_temperature: f64,
}

impl SamplerSettingFacts {
    fn new(cfg: &SamplerConfig, requested_temperature: f64, step: f64) -> Self {
        Self {
            top_k: cfg.k(),
            scale_q16: cfg.scale_q16(),
            requested_temperature,
            effective_temperature: cfg.effective_temperature(step),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplingGateReport {
    pub setting: SamplerSettingFacts,
    pub rom_bytes: usize,
    pub driver_bytes: usize,
    pub n_tokens: u16,
    pub combos: usize,
    pub all_sequences_match: bool,
    pub all_health_checks_pass: bool,
    pub mean_m_cycles_per_token: u64,
    pub seconds_per_token_dmg: f64,
    pub runs: Vec<SamplingComboRun>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplingCycleCost {
    /// Argmax-decode mean M-cycles/token measured in this session (one
    /// 256-token run, seed 'T').
    pub argmax_mean_m_cycles_per_token: u64,
    /// Sampling-decode mean M-cycles/token over all gate combos.
    pub sampling_mean_m_cycles_per_token: u64,
    /// The sampling epilogue's measured extra cost.
    pub extra_m_cycles_per_token: i64,
    pub extra_fraction: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QualitativeSample {
    pub file: String,
    pub setting: SamplerSettingFacts,
    pub seed_id: u8,
    pub seed_char: String,
    pub rng_seed: u16,
    pub n_chars: usize,
    /// "host" or "host+rom-first-256-verified".
    pub provenance: String,
    pub sequence_sha256: String,
    #[serde(skip)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplingDecodeReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub upstream_beads: Vec<&'static str>,
    pub git_sha: String,
    pub checkpoint: StateCheckpointFacts,
    pub design: SamplingDesignFacts,
    pub gate: SamplingGateReport,
    pub cycle_cost: SamplingCycleCost,
    pub samples: Vec<QualitativeSample>,
    pub caveats: Vec<String>,
}

/// The pinned default gate setting: top-k 8 at T = 0.8.
pub const GATE_TOP_K: u8 = 8;
pub const GATE_TEMPERATURE: f64 = 0.8;

/// Qualitative sample settings `(temperature, top_k)`.
pub const SAMPLE_SETTINGS: [(f64, u8); 3] = [(1.0, 8), (0.8, 8), (0.6, 4)];

/// Run every phase and assemble the evidence report.
pub fn run_sampling_bringup(
    repo_root: &Path,
    export_dir_rel: &str,
) -> Result<SamplingDecodeReport, OneTokenError> {
    let export_dir = repo_root.join(export_dir_rel);
    let bundle = load_state_checkpoint(&export_dir)?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    let step = lowered.logit_dequant_step();

    let gate_cfg = SamplerConfig::from_temperature(GATE_TOP_K, step, GATE_TEMPERATURE)
        .map_err(|e| OneTokenError::Model(format!("gate sampler config: {e}")))?;

    // Phase 2: ROM gate over the (seed_byte, rng_seed) combos.
    let rom = build_state_multi_token_sampling_rom(&lowered, SAMPLING_GENERATION_TOKENS, &gate_cfg)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let mut runs = Vec::new();
    for &(seed, rng_seed) in &SAMPLING_GATE_COMBOS {
        runs.push(run_sampling_combo(
            &rom, &lowered, &gate_cfg, seed, rng_seed,
        )?);
    }
    let all_sequences_match = runs.iter().all(|r| r.sequences_match);
    let all_health_checks_pass = runs.iter().all(SamplingComboRun::all_checks_pass);
    let sampling_mean = runs.iter().map(|r| r.cycles.mean).sum::<u64>()
        / u64::try_from(runs.len().max(1)).expect("run count fits u64");
    let gate = SamplingGateReport {
        setting: SamplerSettingFacts::new(&gate_cfg, GATE_TEMPERATURE, step),
        rom_bytes: rom.rom.len(),
        driver_bytes: rom.driver_bytes,
        n_tokens: rom.n_tokens,
        combos: runs.len(),
        all_sequences_match,
        all_health_checks_pass,
        mean_m_cycles_per_token: sampling_mean,
        seconds_per_token_dmg: sampling_mean as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        runs,
    };

    // Phase 3: argmax baseline cycles measured in the same session.
    let argmax_rom = build_state_multi_token_rom(&lowered, SAMPLING_GENERATION_TOKENS)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let argmax_run = crate::stateful::run_state_seed_generation(&argmax_rom, &lowered, 19)?;
    let argmax_mean = argmax_run.cycles.mean;
    let cycle_cost = SamplingCycleCost {
        argmax_mean_m_cycles_per_token: argmax_mean,
        sampling_mean_m_cycles_per_token: sampling_mean,
        extra_m_cycles_per_token: i64::try_from(sampling_mean).unwrap_or(i64::MAX)
            - i64::try_from(argmax_mean).unwrap_or(0),
        extra_fraction: (sampling_mean as f64 - argmax_mean as f64) / argmax_mean as f64,
    };

    // Phase 4: qualitative 512-char samples. The first setting/seed is
    // ROM-verified over its first 256 tokens (the ring capacity).
    let mut samples = Vec::new();
    for (idx, &(temperature, k)) in SAMPLE_SETTINGS.iter().enumerate() {
        let cfg = SamplerConfig::from_temperature(k, step, temperature)
            .map_err(|e| OneTokenError::Model(format!("sample sampler config: {e}")))?;
        let seed = 19u8; // 'T'
        let rng_seed = 0x5EED;
        let host = sampling_host_generate(&lowered, &cfg, seed, rng_seed, 512);
        let mut provenance = "host".to_string();
        if idx == 0 {
            let sample_rom =
                build_state_multi_token_sampling_rom(&lowered, SAMPLING_GENERATION_TOKENS, &cfg)
                    .map_err(|e| OneTokenError::Rom(e.to_string()))?;
            let run = run_sampling_combo(&sample_rom, &lowered, &cfg, seed, rng_seed)?;
            if !run.sequences_match {
                return Err(OneTokenError::Emulator(format!(
                    "qualitative sample ROM diverged from host at {:?}",
                    run.first_divergence_index
                )));
            }
            provenance = "host+rom-first-256-verified".to_string();
        }
        samples.push(QualitativeSample {
            file: format!(
                "sample_T{}_k{}_seed_{:02}.txt",
                format!("{temperature:.2}").replace('.', "p"),
                k,
                seed
            ),
            setting: SamplerSettingFacts::new(&cfg, temperature, step),
            seed_id: seed,
            seed_char: id_to_char(seed).to_string(),
            rng_seed,
            n_chars: host.sequence.len(),
            provenance,
            sequence_sha256: sha256(&host.sequence).to_hex(),
            text: render_char_sample(&host.sequence),
        });
    }

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let lut = build_exp2_lut();
    Ok(SamplingDecodeReport {
        schema: "sampling_decode_bringup.v1",
        bead: "bd-2mjkd",
        upstream_beads: vec!["bd-x5l2s", "bd-29ai4", "bd-a4du"],
        git_sha,
        checkpoint: StateCheckpointFacts {
            export_dir: export_dir_rel.to_string(),
            manifest_schema: bundle.manifest_schema,
            manifest_sha256: bundle.manifest_sha256,
            trainer_git_sha: bundle.manifest_git_sha,
            tensors_verified_sha256: bundle.tensors_verified,
        },
        design: SamplingDesignFacts {
            rng: "XorShift16 (planv0 RngSpec::XorShift16), shift triple (7,9,8), \
                  full period 65535, seed 0 canonicalized to 1, one draw per token"
                .to_string(),
            rng_shifts: XORSHIFT16_SHIFTS,
            exp_lut: "256 x u8, lut[u] = round_ties_even(255 * 2^(-u/16)); zero for u >= 144"
                .to_string(),
            exp_lut_alpha: EXP2_LUT_ALPHA,
            exp_lut_sha256: sha256(lut).to_hex(),
            lut_index_rule: "u = min(255, (d * scale_q16 + 0x8000) >> 16), d = max_logit - logit \
                             in raw i24 logit units (round-half-up Q16 multiply)"
                .to_string(),
            threshold_rule: "threshold = (r * total_weight) >> 16 (truncating); pick the first \
                             candidate in selection order whose cumulative weight strictly \
                             exceeds it"
                .to_string(),
            selection_rule: "top-k by k partial scans; pass 0 is the deployed argmax rule; later \
                             passes skip used ids; ties go to the lower id"
                .to_string(),
            temperature_rule: "scale_q16 = round_ties_even(65536 * 16 * logit_step / (T * ln 2)) \
                               folded at build time"
                .to_string(),
            logit_dequant_step: step,
        },
        gate,
        cycle_cost,
        samples,
        caveats: vec![
            "The output ring caps on-device runs at 256 tokens, so 512-char samples are \
             host-generated; the ROM-verified sample's first 256 ids are byte-identical to an \
             on-device run with the same (seed, rng_seed), and tokens 257..512 come from the \
             same host evaluator that produced those verified ids."
                .to_string(),
            "The scaled-multiply draw makes each candidate's probability deviate from \
             weight/total by at most total/65536 (~3% relative at total ~2040); this is \
             deterministic and identical on host and device."
                .to_string(),
            "Successive XorShift16 values are a fixed permutation of 1..=65535, not \
             independent draws; adequate for qualitative decoding variety, not for \
             statistical workloads."
                .to_string(),
            "Sample quality must be judged honestly: sampling escapes greedy argmax loops, \
             but the model itself is a 4-block d64 bring-up checkpoint (~3.3 bpc), so text is \
             English-like at best, not coherent prose."
                .to_string(),
        ],
    })
}

/// Render the report README (generated, not hand-written).
#[must_use]
pub fn sampling_report_to_markdown(report: &SamplingDecodeReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# Sampling decode bring-up ({})", report.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Integer top-k/temperature sampling decode for the stateful arm-B checkpoint: the \
         exp-LUT + XorShift16 design that the planv0 2026-07-04 amendment section 3 decode pin \
         requires before `DecodeMode::TopKTemperature` can exist (F-G3, bd-a4du; this run is \
         bead {}). Host semantics live in `gbf-kernel/src/decode.rs`; the ROM epilogue must \
         reproduce them byte-exactly. Generated by `cargo run -p gbf-bench --bin \
         sampling-decode`; every number below is program output at git `{}`.",
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
    let _ = writeln!(out, "## Pinned design");
    let _ = writeln!(out);
    let d = &report.design;
    let _ = writeln!(out, "- RNG: {}", d.rng);
    let _ = writeln!(
        out,
        "- Exp LUT: {} (sha256 `{}`)",
        d.exp_lut,
        &d.exp_lut_sha256[..16]
    );
    let _ = writeln!(out, "- LUT index: {}", d.lut_index_rule);
    let _ = writeln!(out, "- Selection: {}", d.selection_rule);
    let _ = writeln!(out, "- Draw: {}", d.threshold_rule);
    let _ = writeln!(
        out,
        "- Temperature: {} (logit step {:.6e})",
        d.temperature_rule, d.logit_dequant_step
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "## ROM gate — on-device sampling generation, byte-exact vs host"
    );
    let _ = writeln!(out);
    let g = &report.gate;
    let _ = writeln!(
        out,
        "- Setting: top-k {}, requested T {} (scale_q16 {}, effective T {:.4})",
        g.setting.top_k,
        g.setting.requested_temperature,
        g.setting.scale_q16,
        g.setting.effective_temperature
    );
    let _ = writeln!(
        out,
        "- **Sequences: {}** — {}/{} (seed_byte, rng_seed) combos x {} tokens byte-identical \
         to the host integer sampler",
        if g.all_sequences_match {
            "PASS"
        } else {
            "FAIL"
        },
        g.runs.iter().filter(|r| r.sequences_match).count(),
        g.runs.len(),
        g.n_tokens
    );
    let _ = writeln!(
        out,
        "- **Health: {}** — SP home each token, untouched WRAM regions unchanged, stable \
         cycles, first/last-token forward dumps and sampled ids byte-exact, done flag set",
        if g.all_health_checks_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| seed id | char | rng seed | sequence match | first/last dumps | sampled ids | cycles median | max/min | SP | WRAM | sample |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---|---|");
    for run in &g.runs {
        let _ = writeln!(
            out,
            "| {} | `{}` | 0x{:04X} | {} | {}/{} | {} | {} | {:.5} | {} | {} | `{}` |",
            run.seed_id,
            id_to_char(run.seed_id).escape_default(),
            run.rng_seed,
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
            if run.sampled_id_matches_first_and_last {
                "yes"
            } else {
                "NO"
            },
            run.cycles.median,
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
    let _ = writeln!(out, "## Cycle cost of the sampling epilogue");
    let _ = writeln!(out);
    let cc = &report.cycle_cost;
    let _ = writeln!(
        out,
        "- Argmax decode (same session, 256 tokens): **{} M-cycles/token**",
        cc.argmax_mean_m_cycles_per_token
    );
    let _ = writeln!(
        out,
        "- Sampling decode: **{} M-cycles/token** ({:+} M-cycles/token, {:+.4}% — the forward \
         pass dominates; the sampler adds k logit scans, one LUT multiply per candidate, one \
         RNG step, and the cumulative walk)",
        cc.sampling_mean_m_cycles_per_token,
        cc.extra_m_cycles_per_token,
        cc.extra_fraction * 100.0
    );
    let _ = writeln!(
        out,
        "- Seconds/token on DMG at the gate setting: {:.3}",
        report.gate.seconds_per_token_dmg
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Qualitative samples (512 chars)");
    let _ = writeln!(out);
    for s in &report.samples {
        let _ = writeln!(
            out,
            "- `{}`: T {} (effective {:.4}), top-k {}, seed `{}`, rng 0x{:04X}, {} chars, {}",
            s.file,
            s.setting.requested_temperature,
            s.setting.effective_temperature,
            s.setting.top_k,
            s.seed_char.escape_default(),
            s.rng_seed,
            s.n_chars,
            s.provenance
        );
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

    /// Full-stack smoke on a synthetic checkpoint: the sampling ROM's
    /// on-device loop (top-k scans, LUT weights, RNG, cumulative pick,
    /// feedback) must reproduce the host sampler byte-exactly, including
    /// the zero rng seed canonicalization. Same machinery as the
    /// real-checkpoint gate.
    #[test]
    fn sampling_rom_matches_host_on_synthetic_model() {
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = SamplerConfig::new(8, 2253).expect("valid sampler");
        let rom = build_state_multi_token_sampling_rom(&lowered, 6, &cfg).expect("builds");
        for &(seed, rng_seed) in &[(19u8, 0xBEEFu16), (42, 0x0000)] {
            let run = run_sampling_combo(&rom, &lowered, &cfg, seed, rng_seed).expect("runs");
            assert!(
                run.sequences_match,
                "seed {seed} rng {rng_seed:#06x}: ROM diverged at {:?} ({:?})",
                run.first_divergence_index, run.checkpoint_mismatches
            );
            assert!(
                run.first_token_checkpoints_byte_exact
                    && run.last_token_checkpoints_byte_exact
                    && run.sampled_id_matches_first_and_last,
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
            assert_eq!(run.rom_sequence.len(), 6);
        }
    }

    /// V2 dispatch lowering must reproduce the sampling ROM byte-exactly too:
    /// the sampler routines sit on top of the multi-token driver, so this also
    /// confirms the V2 shared handler still fits bank 0 with the sampler.
    #[test]
    fn v2_sampling_rom_matches_host_on_synthetic_model() {
        use gbf_kernel::asm_impl_state::{
            WeightLowering, build_state_multi_token_sampling_rom_lowered,
        };
        let ck = synthetic_state_checkpoint(21);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = SamplerConfig::new(8, 2253).expect("valid sampler");
        let rom = build_state_multi_token_sampling_rom_lowered(
            &lowered,
            6,
            &cfg,
            WeightLowering::V2Dispatch,
        )
        .expect("v2 sampling ROM builds");
        for &(seed, rng_seed) in &[(19u8, 0xBEEFu16), (42, 0x0000)] {
            let run = run_sampling_combo(&rom, &lowered, &cfg, seed, rng_seed).expect("runs");
            assert!(
                run.sequences_match,
                "V2 seed {seed} rng {rng_seed:#06x}: ROM diverged at {:?} ({:?})",
                run.first_divergence_index, run.checkpoint_mismatches
            );
            assert!(
                run.first_token_checkpoints_byte_exact
                    && run.last_token_checkpoints_byte_exact
                    && run.sampled_id_matches_first_and_last,
                "V2 dump mismatches {:?}",
                run.checkpoint_mismatches
            );
            assert!(run.done_flag_set);
        }
    }

    /// d192 sampling + shell drivers must still fit bank 0 (0x150..0x4000)
    /// under V2 — they carry the most bank-0 code (sampler / UI) on top of the
    /// shared handler.
    #[test]
    fn v2_d192_sampling_and_shell_fit_bank0() {
        use gbf_kernel::asm_impl_shell::{build_state_shell_rom_lowered, synthetic_font_tiles};
        use gbf_kernel::asm_impl_state::{
            WeightLowering, build_state_multi_token_sampling_rom_lowered,
        };
        use gbf_kernel::state_model_ref::{StateTopology, synthetic_state_checkpoint_with};
        let ck = synthetic_state_checkpoint_with(StateTopology::D192, 5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = SamplerConfig::new(8, 2253).expect("valid sampler");
        let samp = build_state_multi_token_sampling_rom_lowered(
            &lowered,
            32,
            &cfg,
            WeightLowering::V2Dispatch,
        )
        .expect("v2 d192 sampling ROM builds within bank 0");
        assert!(
            samp.driver_bytes < 0x4000 - 0x150,
            "v2 d192 sampling driver {} must fit bank 0",
            samp.driver_bytes
        );
        let font = synthetic_font_tiles();
        let shell =
            build_state_shell_rom_lowered(&lowered, &cfg, 6, &font, WeightLowering::V2Dispatch)
                .expect("v2 d192 shell ROM builds within bank 0");
        assert!(
            shell.driver_bytes < 0x4000 - 0x150,
            "v2 d192 shell driver {} must fit bank 0",
            shell.driver_bytes
        );
    }

    /// The host mirror is deterministic and differs across rng seeds
    /// (generically) while the argmax path is seed-independent.
    #[test]
    fn host_mirror_is_deterministic_per_seed() {
        let ck = synthetic_state_checkpoint(5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = SamplerConfig::new(8, 4000).expect("valid sampler");
        let a = sampling_host_generate(&lowered, &cfg, 19, 0xBEEF, 32);
        let b = sampling_host_generate(&lowered, &cfg, 19, 0xBEEF, 32);
        assert_eq!(a.sequence, b.sequence);
        let c = sampling_host_generate(&lowered, &cfg, 19, 0x1234, 32);
        assert_ne!(
            a.sequence, c.sequence,
            "different rng seeds should generically sample different sequences"
        );
    }
}
