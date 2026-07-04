//! Compiled-ROM gate (bd-1skgm): the ROM produced by the `gbf compile`
//! pipeline (`gbf_codegen::compile`) must reproduce the committed
//! multi-token behavior — the 4-seed, 256-token on-device generation must be
//! byte-identical to the host canonical integer evaluator, with every
//! health check green. A compiler that produces a ROM failing this gate is
//! not done.
//!
//! This reuses the bd-2gc6p gate machinery ([`crate::multi_token`])
//! unchanged; the only difference is that the ROM under test comes out of
//! the compiler dataflow (checkpoint export -> ArtifactCore -> lowering
//! middle -> kernel selection -> ROM backend) instead of the hand-wired
//! bring-up call. When the committed multi-token evidence
//! (`docs/experiments/multi-token/report.json`) is present, the gate
//! additionally cross-checks each seed's generated-sequence sha256 against
//! it, proving the compiled ROM reproduces the committed behavior exactly.

use std::path::Path;

use gbf_codegen::compile::{BuildReport, CompileError, CompileOptions, compile_checkpoint_export};
use serde::Serialize;

use crate::multi_token::{SeedRun, run_seed_generation};
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};

/// Committed multi-token evidence file used for the cross-check.
pub const COMMITTED_EVIDENCE_REL: &str = "docs/experiments/multi-token/report.json";

#[derive(Debug)]
pub enum CompileGateError {
    Compile(CompileError),
    Gate(OneTokenError),
}

impl std::fmt::Display for CompileGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(error) => write!(f, "compile: {error}"),
            Self::Gate(error) => write!(f, "gate: {error}"),
        }
    }
}

impl std::error::Error for CompileGateError {}

/// Per-seed cross-check against the committed multi-token evidence.
#[derive(Debug, Clone, Serialize)]
pub struct CommittedSequenceCheck {
    pub seed: u8,
    /// sha256 of the committed evidence's on-device sequence for this seed
    /// (absent if the evidence file or seed entry is unavailable).
    pub committed_sha256: Option<String>,
    /// sha256 of the compiled ROM's on-device sequence.
    pub compiled_sha256: String,
    pub matches: Option<bool>,
}

/// The complete compiled-ROM gate report (program-generated evidence).
#[derive(Debug, Clone, Serialize)]
pub struct CompileGateReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub git_sha: String,
    /// The compiler's own build report for the ROM under test.
    pub build_report: BuildReport,
    pub seeds: Vec<u8>,
    /// Primary gate: on-device sequences byte-identical to the host
    /// evaluator for every seed.
    pub all_sequences_match: bool,
    /// Health checks (SP home, WRAM stability, cycle stability, first/last
    /// dump agreement, done flag) green for every seed.
    pub all_health_checks_pass: bool,
    /// Cross-check against the committed bd-2gc6p evidence, when present.
    pub committed_evidence_file: &'static str,
    pub committed_evidence_checks: Vec<CommittedSequenceCheck>,
    /// `Some(true)` when every seed matched the committed evidence,
    /// `None` when the evidence file was unavailable.
    pub all_match_committed_evidence: Option<bool>,
    pub mean_m_cycles_per_token: u64,
    pub seconds_per_token_dmg: f64,
    pub runs: Vec<SeedRun>,
}

impl CompileGateReport {
    /// The gate: sequences match the host, health checks pass, and (when the
    /// committed evidence is present) the sequences match it too.
    #[must_use]
    pub fn gate_passes(&self) -> bool {
        self.all_sequences_match
            && self.all_health_checks_pass
            && self.all_match_committed_evidence != Some(false)
    }
}

fn committed_sequence_shas(repo_root: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(repo_root.join(COMMITTED_EVIDENCE_REL)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn committed_sha_for_seed(evidence: Option<&serde_json::Value>, seed: u8) -> Option<String> {
    let runs = evidence?.get("runs")?.as_array()?;
    runs.iter()
        .find(|run| run.get("seed").and_then(serde_json::Value::as_u64) == Some(u64::from(seed)))?
        .get("rom_sequence_sha256")?
        .as_str()
        .map(str::to_string)
}

/// Compile the checkpoint export through the full compiler dataflow and run
/// the multi-token generation gate over `seeds`.
pub fn run_compile_gate(
    repo_root: &Path,
    export_dir_rel: &str,
    seeds: &[u8],
    n_tokens: u16,
) -> Result<CompileGateReport, CompileGateError> {
    let compiled = compile_checkpoint_export(
        &repo_root.join(export_dir_rel),
        &CompileOptions { n_tokens },
    )
    .map_err(CompileGateError::Compile)?;

    let mut runs = Vec::new();
    for &seed in seeds {
        runs.push(
            run_seed_generation(&compiled.rom, &compiled.int_model, seed)
                .map_err(CompileGateError::Gate)?,
        );
    }
    let all_sequences_match = runs.iter().all(|r| r.sequences_match);
    let all_health_checks_pass = runs.iter().all(SeedRun::all_checks_pass);
    let mean_m_cycles_per_token = runs.iter().map(|r| r.cycles.mean).sum::<u64>()
        / u64::try_from(runs.len().max(1)).expect("run count fits u64");

    // Cross-check vs the committed evidence (only meaningful at the
    // committed generation length).
    let evidence = if n_tokens == 256 {
        committed_sequence_shas(repo_root)
    } else {
        None
    };
    let committed_evidence_checks: Vec<CommittedSequenceCheck> = runs
        .iter()
        .map(|run| {
            let committed = committed_sha_for_seed(evidence.as_ref(), run.seed);
            let matches = committed
                .as_ref()
                .map(|sha| *sha == run.rom_sequence_sha256);
            CommittedSequenceCheck {
                seed: run.seed,
                committed_sha256: committed,
                compiled_sha256: run.rom_sequence_sha256.clone(),
                matches,
            }
        })
        .collect();
    let all_match_committed_evidence = if committed_evidence_checks
        .iter()
        .all(|c| c.matches.is_none())
    {
        None
    } else {
        Some(
            committed_evidence_checks
                .iter()
                .all(|c| c.matches == Some(true)),
        )
    };

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(CompileGateReport {
        schema: "gbf_compile_gate.v1",
        bead: "bd-1skgm",
        git_sha,
        build_report: compiled.report,
        seeds: seeds.to_vec(),
        all_sequences_match,
        all_health_checks_pass,
        committed_evidence_file: COMMITTED_EVIDENCE_REL,
        committed_evidence_checks,
        all_match_committed_evidence,
        mean_m_cycles_per_token,
        seconds_per_token_dmg: mean_m_cycles_per_token as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        runs,
    })
}

/// Render the evidence README (generated, not hand-written).
#[must_use]
pub fn compile_gate_report_to_markdown(report: &CompileGateReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# `gbf compile` pipeline gate ({})", report.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "First model ROM produced by the compiler dataflow (bd-1skgm): checkpoint export -> \
         sha256-verified `ArtifactCore` import -> lowering middle (`lower_infer` narrow model \
         IR, `lower_quant` v0 integer contract, `legalize` device bounds, `kernel_select` \
         V3 weights-as-code) -> banked MBC5 ROM backend -> `gbf compile` CLI. Generated by \
         `cargo run --release -p gbf-bench --bin compile-gate`; every number below is program \
         output at git `{}`.",
        report.git_sha
    );
    let _ = writeln!(out);
    let b = &report.build_report;
    let _ = writeln!(out, "## Compiled artifact");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Checkpoint `{}` (trainer git `{}`), {} tensors sha256-verified, manifest sha256 `{}`",
        b.checkpoint.schema,
        b.checkpoint.trainer_git_sha,
        b.checkpoint.tensors_verified_sha256,
        &b.checkpoint.manifest_sha256[..16],
    );
    let _ = writeln!(
        out,
        "- Artifact core semantic hash `{}` ({} canonical tensors); sequence semantics: {}",
        b.artifact.semantic_hash,
        b.artifact.tensor_count,
        b.checkpoint.sequence_semantics_placeholder
    );
    let _ = writeln!(
        out,
        "- Program: d_model {}, d_ff {}, {} blocks, vocab {}, {} ops; weight zeros {} permille",
        b.program.d_model,
        b.program.d_ff,
        b.program.n_blocks,
        b.program.vocab,
        b.program.ops.len(),
        b.program.weight_zero_permille
    );
    let _ = writeln!(
        out,
        "- Legalization: {}/{} checks passed; kernel plan: {} selections (block matvecs -> \
         V3 weights-as-code, tied head -> lane-major i8 product-LUT)",
        b.legalization.checks.iter().filter(|c| c.ok).count(),
        b.legalization.checks.len(),
        b.kernel_plan.len()
    );
    let _ = writeln!(
        out,
        "- ROM: {} bytes ({} banks), driver {} B, weight code {} B in {} chunks, tables {} B, \
         {}-token generation loop",
        b.rom.rom_bytes,
        b.rom.bank_count,
        b.rom.driver_bytes,
        b.rom.weight_code_bytes,
        b.rom.weight_chunk_count,
        b.rom.table_bytes,
        b.rom.n_tokens
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Gate");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- **Sequences: {}** — {}/{} seeds produced a {}-byte on-device sequence identical to \
         the host integer evaluator",
        if report.all_sequences_match {
            "PASS"
        } else {
            "FAIL"
        },
        report.runs.iter().filter(|r| r.sequences_match).count(),
        report.runs.len(),
        b.rom.n_tokens
    );
    let _ = writeln!(
        out,
        "- **Health: {}** — SP home every token, untouched WRAM unchanged, cycles stable, \
         first/last-token dumps byte-exact, done flag set",
        if report.all_health_checks_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(
        out,
        "- **Committed-evidence cross-check: {}** — per-seed sequence sha256 vs `{}`",
        match report.all_match_committed_evidence {
            Some(true) => "PASS (byte-identical to the bd-2gc6p evidence)",
            Some(false) => "FAIL",
            None => "unavailable",
        },
        report.committed_evidence_file
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| seed | sequence match | committed sha match | first/last dumps | cycles/token (median) | SP home | WRAM clean |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for (run, check) in report.runs.iter().zip(&report.committed_evidence_checks) {
        let _ = writeln!(
            out,
            "| 0x{:02X} | {} | {} | {}/{} | {} | {} | {} |",
            run.seed,
            if run.sequences_match { "yes" } else { "NO" },
            match check.matches {
                Some(true) => "yes",
                Some(false) => "NO",
                None => "n/a",
            },
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
            run.cycles.median,
            if run.sp_home_every_token { "yes" } else { "NO" },
            if run.wram_untouched_regions_ok {
                "yes"
            } else {
                "NO"
            },
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Mean over all seeds and tokens: **{} M-cycles/token** = {:.3} s/token on DMG",
        report.mean_m_cycles_per_token, report.seconds_per_token_dmg
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## What is real vs still stubbed");
    let _ = writeln!(out);
    let _ = writeln!(out, "Stages executed with real dataflow on this path:");
    let _ = writeln!(out);
    for stage in &b.stage_coverage.real_dataflow {
        let _ = writeln!(out, "- {stage}");
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Generic pipeline stages **not** on this path (no real producer yet):"
    );
    let _ = writeln!(out);
    for stage in &b.stage_coverage.not_wired {
        let _ = writeln!(out, "- {stage}");
    }
    let _ = writeln!(out);
    for note in &b.stage_coverage.notes {
        let _ = writeln!(out, "- {note}");
    }
    out
}
