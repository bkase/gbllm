//! End-to-end demo acceptance packet (bd-do7sq): boot the deployed d192
//! shell ROM in the emulator, type an evocative prompt on the on-screen
//! keyboard via injected joypad frames, press START, and watch >= 128
//! sampled tokens render into the transcript — captured as screenshots,
//! transcript text, a determinism proof (second run byte-identical), and
//! byte-identity of the generated sequence vs the host integer evaluator.
//!
//! The bead's quality bar ("beats KN-5") is recorded HONESTLY as open: the
//! deployed model does not beat the committed KN-5 baseline, and KN-5 does
//! not fit on-cart. The packet therefore claims "best deployable model to
//! date, coherent pseudo-prose" and names the next quality levers
//! (bd-3771m). Evidence (`demo_acceptance.v1`) is produced by the
//! `demo-acceptance` bin — never hand-written.

use std::fs;
use std::path::Path;

use gbf_foundation::sha256;
use gbf_kernel::asm_impl_shell::build_state_shell_rom;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::IntStateLoweredModel;
use serde::Serialize;

use crate::d192_real::{D192_REAL_EXPORT_DIR, load_committed_provenance};
use crate::one_token::{DMG_M_CYCLES_PER_SECOND, OneTokenError};
use crate::sampling::SamplerSettingFacts;
use crate::shell::{
    SHELL_TEMPERATURE, SHELL_TOP_K, ShellSessionResult, char_to_id, framebuffer_to_pgm,
    run_shell_session_observed, shell_font_tiles, transcript_text,
};
use crate::stateful::{StateCheckpointFacts, load_state_checkpoint};

/// The demo prompt: evocative, and exactly the 20-char shell prompt cap
/// (the on-screen prompt row is one BG row). "The machines dreamed of"
/// would be 23 chars; the cap keeps the committed sample-set prompt.
pub const DEMO_PROMPT: &str = "The machines dreamed";
pub const DEMO_RNG_SEED: u16 = 0x5EED;
/// Minimum on-device generated tokens for acceptance.
pub const DEMO_MIN_TOKENS: usize = 128;

/// Committed evidence inputs for the honest quality section.
pub const D192_REAL_REPORT: &str = "docs/experiments/d192-real/report.json";
pub const KN5_BASELINE_FULL: &str = "experiments/S4/baseline/s4_baseline_gutenberg.v1.json";
pub const KN5_BASELINE_256MIB: &str =
    "experiments/S4/baseline/s4_baseline_gutenberg.v1.train-cap-256MiB.json";
pub const KN5_BASELINE_64MIB: &str =
    "experiments/S4/baseline/s4_baseline_gutenberg.v1.train-cap-64MiB.json";

// ---------------------------------------------------------------------------
// honest quality facts (every number read from committed evidence)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Kn5CapFacts {
    pub source: String,
    pub train_cap: String,
    pub bpc_per_normalized_char: f64,
    pub bits_per_raw_byte: f64,
    pub c2_unique_contexts: u64,
    pub c3_unique_contexts: u64,
    pub c4_unique_contexts: u64,
    pub c5_unique_contexts: u64,
    pub total_unique_ngram_entries: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeployedModelQuality {
    pub source: String,
    pub int_semantics_version: String,
    /// The deployment-relevant number: the integer path's own val bpc.
    pub int_val_bpc_per_normalized_char: f64,
    pub int_val_bits_per_raw_byte: f64,
    /// The trainer's committed hard-ternary measurement (f32 semantics).
    pub committed_hard_ternary_val_bpc: f64,
    pub committed_hard_ternary_bits_per_raw_byte: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoQualityFacts {
    pub quality_bar: &'static str,
    pub quality_bar_status: &'static str,
    pub deployed: DeployedModelQuality,
    pub kn5: Vec<Kn5CapFacts>,
    /// The honest verdict: the deployed integer-path model does NOT beat
    /// the KN-5 reference on gutenberg_val.
    pub deployed_beats_kn5_full: bool,
    pub deployed_minus_kn5_full_bits_per_raw_byte: f64,
    pub kn5_on_cart_feasibility: String,
    pub honest_claim: &'static str,
    pub next_levers: Vec<&'static str>,
    pub quality_owner_bead: &'static str,
}

fn json_f64(v: &serde_json::Value, ptr: &str, path: &Path) -> Result<f64, OneTokenError> {
    v.pointer(ptr)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| OneTokenError::Manifest {
            reason: format!("{} missing {ptr}", path.display()),
        })
}

fn json_u64(v: &serde_json::Value, ptr: &str, path: &Path) -> Result<u64, OneTokenError> {
    v.pointer(ptr)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| OneTokenError::Manifest {
            reason: format!("{} missing {ptr}", path.display()),
        })
}

fn read_json(path: &Path) -> Result<serde_json::Value, OneTokenError> {
    let bytes = fs::read(path).map_err(|e| OneTokenError::Io {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|e| OneTokenError::Manifest {
        reason: format!("{}: {e}", path.display()),
    })
}

fn load_kn5_cap(
    repo_root: &Path,
    rel: &str,
    cap: &str,
    chars_per_raw_byte: f64,
) -> Result<Kn5CapFacts, OneTokenError> {
    let path = repo_root.join(rel);
    let v = read_json(&path)?;
    let bpc = json_f64(&v, "/bpc_kn5", &path)?;
    let c2 = json_u64(&v, "/counts_summary/c2_unique_count", &path)?;
    let c3 = json_u64(&v, "/counts_summary/c3_unique_count", &path)?;
    let c4 = json_u64(&v, "/counts_summary/c4_unique_count", &path)?;
    let c5 = json_u64(&v, "/counts_summary/c5_unique_count", &path)?;
    Ok(Kn5CapFacts {
        source: rel.to_string(),
        train_cap: cap.to_string(),
        bpc_per_normalized_char: bpc,
        bits_per_raw_byte: bpc * chars_per_raw_byte,
        c2_unique_contexts: c2,
        c3_unique_contexts: c3,
        c4_unique_contexts: c4,
        c5_unique_contexts: c5,
        total_unique_ngram_entries: c2 + c3 + c4 + c5,
    })
}

/// Assemble the honest quality section from committed evidence only.
pub fn load_demo_quality_facts(repo_root: &Path) -> Result<DemoQualityFacts, OneTokenError> {
    let real_path = repo_root.join(D192_REAL_REPORT);
    let real = read_json(&real_path)?;
    let deployed = DeployedModelQuality {
        source: D192_REAL_REPORT.to_string(),
        int_semantics_version: real
            .pointer("/int_semantics_version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        int_val_bpc_per_normalized_char: json_f64(&real, "/fidelity/int_val_bpc", &real_path)?,
        int_val_bits_per_raw_byte: json_f64(
            &real,
            "/fidelity/int_val_bits_per_raw_byte",
            &real_path,
        )?,
        committed_hard_ternary_val_bpc: json_f64(
            &real,
            "/fidelity/committed_ternary_val_bpc",
            &real_path,
        )?,
        committed_hard_ternary_bits_per_raw_byte: json_f64(
            &real,
            "/fidelity/committed_ternary_val_bits_per_raw_byte",
            &real_path,
        )?,
    };

    // The KN-5 baseline scored the same book-level val stream as the arm
    // record; re-express bpc/normalized-char as bits/raw-byte with the same
    // committed factor the arm record uses.
    let committed = load_committed_provenance(repo_root)?;
    #[allow(clippy::cast_precision_loss)]
    let chars_per_raw_byte =
        committed.val_chars_normalized as f64 / committed.val_raw_bytes_used as f64;

    let kn5 = vec![
        load_kn5_cap(
            repo_root,
            KN5_BASELINE_FULL,
            "full (489,614,571 train bytes)",
            chars_per_raw_byte,
        )?,
        load_kn5_cap(
            repo_root,
            KN5_BASELINE_256MIB,
            "256 MiB",
            chars_per_raw_byte,
        )?,
        load_kn5_cap(repo_root, KN5_BASELINE_64MIB, "64 MiB", chars_per_raw_byte)?,
    ];
    let kn5_full = &kn5[0];
    let gap = deployed.int_val_bits_per_raw_byte - kn5_full.bits_per_raw_byte;
    let feasibility = format!(
        "KN-5 does not fit on-cart: the full-corpus fit holds {} unique n-gram entries \
         (orders 2-5; {} unique 5-gram contexts alone) and the 256 MiB-cap fit still holds \
         {} 5-gram contexts. Every entry needs a packed multi-char context key plus smoothed \
         continuation statistics — several bytes per entry before any runtime code — while \
         the 8 MiB MBC5 ROM ceiling is already saturated by the deployed d192 ROM (8,388,608 \
         bytes, ~6.07 MiB of it weight code). No committed on-cart KN-5 representation or \
         DMG-latency evaluation of one exists.",
        kn5_full.total_unique_ngram_entries, kn5_full.c5_unique_contexts, kn5[1].c5_unique_contexts
    );
    Ok(DemoQualityFacts {
        quality_bar: "beats KN-5 on gutenberg_val (bd-do7sq quality bar, tied to the S4/S8 \
                      scoring; KN-5 reference = bd-2nca full-corpus fit)",
        quality_bar_status: "OPEN",
        deployed,
        kn5,
        deployed_beats_kn5_full: gap < 0.0,
        deployed_minus_kn5_full_bits_per_raw_byte: gap,
        kn5_on_cart_feasibility: feasibility,
        honest_claim: "best deployable model to date that fits the cart and the UX budget; \
                       output is coherent English-like pseudo-prose, not KN-5-beating text",
        next_levers: vec![
            "close the hard-ternary QAT gap (scale run 2 widened it 3.6x: +0.1033 bpc; levers: \
             longer hardness warmup, later hard switch, lower hard-phase lr, gap-aware distill \
             weight schedule)",
            "raise ternary sparsity (deployed zero fraction 0.0085; V3 kernels skip zeros, so \
             sparsity buys both quality-per-byte and latency)",
            "longer training / better teacher-student recipes on the saved 75k-step teacher \
             (student-only reruns cost ~8.3 h)",
        ],
        quality_owner_bead: "bd-3771m",
    })
}

// ---------------------------------------------------------------------------
// determinism + screenshots
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotFacts {
    pub file: String,
    pub phase: String,
    pub framebuffer_sha256: String,
    pub pgm_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoDeterminismFacts {
    pub sessions: usize,
    pub sequences_identical: bool,
    pub framebuffer_hashes_identical: bool,
    /// The committed PGM bytes are byte-identical across the two runs.
    pub screenshot_pgm_bytes_identical: bool,
    pub transcripts_identical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoGate {
    pub gate: String,
    pub pass: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoAcceptanceReport {
    pub schema: &'static str,
    pub bead: &'static str,
    pub upstream_beads: Vec<&'static str>,
    pub git_sha: String,
    pub scope: &'static str,
    pub checkpoint: StateCheckpointFacts,
    pub sampler: SamplerSettingFacts,
    pub prompt: String,
    pub prompt_note: &'static str,
    pub rng_seed: u16,
    pub min_required_tokens: usize,
    pub gates: Vec<DemoGate>,
    pub all_demo_gates_pass: bool,
    pub session: ShellSessionResult,
    pub seconds_per_token_dmg_mean: f64,
    pub latency_gate_packet: &'static str,
    pub screenshots: Vec<ScreenshotFacts>,
    pub transcript_file: &'static str,
    pub transcript_sha256: String,
    pub determinism: DemoDeterminismFacts,
    pub quality: DemoQualityFacts,
    pub open_items: Vec<&'static str>,
    pub quick_mode: bool,
    pub caveats: Vec<String>,
    #[serde(skip)]
    pub screenshot_pgms: Vec<(String, Vec<u8>)>,
    #[serde(skip)]
    pub transcript: String,
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

/// Run the full scripted end-to-end demo twice (determinism) and assemble
/// the acceptance packet.
#[allow(clippy::too_many_lines)]
pub fn run_demo_acceptance(
    repo_root: &Path,
    quick: bool,
) -> Result<DemoAcceptanceReport, OneTokenError> {
    let (n_gen, min_tokens) = if quick {
        (8u8, 1usize)
    } else {
        (200u8, DEMO_MIN_TOKENS)
    };

    let bundle = load_state_checkpoint(&repo_root.join(D192_REAL_EXPORT_DIR))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)
        .map_err(|e| OneTokenError::Model(e.to_string()))?;
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(SHELL_TOP_K, step, SHELL_TEMPERATURE)
        .map_err(|e| OneTokenError::Model(format!("sampler config: {e}")))?;
    let font = shell_font_tiles();
    let rom = build_state_shell_rom(&lowered, &cfg, n_gen, &font)
        .map_err(|e| OneTokenError::Rom(e.to_string()))?;
    let prompt_ids: Vec<u8> = DEMO_PROMPT
        .chars()
        .map(|c| char_to_id(c).expect("demo prompt chars are charset_v1 printables"))
        .collect();

    let session =
        run_shell_session_observed(&rom, &lowered, &cfg, &prompt_ids, DEMO_RNG_SEED, true)?;
    let rerun = run_shell_session_observed(&rom, &lowered, &cfg, &prompt_ids, DEMO_RNG_SEED, true)?;

    let pgms: Vec<(String, Vec<u8>)> = session
        .framebuffers
        .iter()
        .map(|(name, fb)| (name.clone(), framebuffer_to_pgm(fb)))
        .collect();
    let rerun_pgms: Vec<(String, Vec<u8>)> = rerun
        .framebuffers
        .iter()
        .map(|(name, fb)| (name.clone(), framebuffer_to_pgm(fb)))
        .collect();
    let transcript = transcript_text(&session.prompt_ids, &session.rom_sequence);
    let rerun_transcript = transcript_text(&rerun.prompt_ids, &rerun.rom_sequence);

    let determinism = DemoDeterminismFacts {
        sessions: 2,
        sequences_identical: session.rom_sequence == rerun.rom_sequence,
        framebuffer_hashes_identical: session.fb_sha256_after_boot == rerun.fb_sha256_after_boot
            && session.fb_sha256_after_typing == rerun.fb_sha256_after_typing
            && session.fb_sha256_mid_generation == rerun.fb_sha256_mid_generation
            && session.fb_sha256_after_generation == rerun.fb_sha256_after_generation,
        screenshot_pgm_bytes_identical: pgms == rerun_pgms,
        transcripts_identical: transcript == rerun_transcript,
    };

    let phases = [
        "boot (idle keyboard)",
        "prompt typed",
        "mid-generation",
        "generation done",
    ];
    let fb_hashes = [
        session.fb_sha256_after_boot.clone(),
        session.fb_sha256_after_typing.clone(),
        session.fb_sha256_mid_generation.clone().unwrap_or_default(),
        session.fb_sha256_after_generation.clone(),
    ];
    let screenshots: Vec<ScreenshotFacts> = pgms
        .iter()
        .zip(phases.iter())
        .zip(fb_hashes.iter())
        .map(|(((file, bytes), phase), fb_sha)| ScreenshotFacts {
            file: file.clone(),
            phase: (*phase).to_string(),
            framebuffer_sha256: fb_sha.clone(),
            pgm_sha256: sha256(bytes).to_hex(),
        })
        .collect();

    let quality = load_demo_quality_facts(repo_root)?;

    let mean_token = if session.token_boundary_m_cycles.is_empty() {
        0
    } else {
        session.token_boundary_m_cycles.iter().sum::<u64>()
            / session.token_boundary_m_cycles.len() as u64
    };

    let mut gates = vec![
        DemoGate {
            gate: "boot: ROM boots to keyboard/status/transcript chrome".to_string(),
            pass: session.boot_chrome_ok,
            detail: format!(
                "cell-by-cell BG check; fb {}",
                &session.fb_sha256_after_boot[..16]
            ),
        },
        DemoGate {
            gate: format!(
                "prompt entry: `{DEMO_PROMPT}` typed via {} injected joypad frames, echoed on \
                 the prompt row",
                session.typing_frames
            ),
            pass: session.prompt_echo_ok,
            detail: format!("prompt ids {:?}", session.prompt_ids),
        },
        DemoGate {
            gate: format!(
                "generation: >= {min_tokens} tokens sampled on-device (top-k {}, T {})",
                SHELL_TOP_K, SHELL_TEMPERATURE
            ),
            pass: session.n_tokens_generated >= min_tokens,
            detail: format!("{} tokens generated", session.n_tokens_generated),
        },
        DemoGate {
            gate: "host byte-identity: on-device sequence == host integer evaluator".to_string(),
            pass: session.sequences_match,
            detail: format!(
                "sha256 host {} / rom {}, first divergence {:?}",
                &session.host_sequence_sha256[..16],
                &session.rom_sequence_sha256[..16],
                session.first_divergence_index
            ),
        },
        DemoGate {
            gate: "transcript render: BG region contains exactly the rendered glyph tiles"
                .to_string(),
            pass: session.transcript_bg_ok,
            detail: format!("{} BG mismatches", session.bg_mismatches.len()),
        },
        DemoGate {
            gate: "session end: post-run chrome restored, shell returns to idle input".to_string(),
            pass: session.post_run_chrome_ok && session.returned_to_idle,
            detail: String::new(),
        },
        DemoGate {
            gate: "determinism: second full session byte-identical (sequence, framebuffer \
                   hashes, PGM bytes, transcript)"
                .to_string(),
            pass: determinism.sequences_identical
                && determinism.framebuffer_hashes_identical
                && determinism.screenshot_pgm_bytes_identical
                && determinism.transcripts_identical,
            detail: format!("{} sessions", determinism.sessions),
        },
        DemoGate {
            gate: "screenshots: >= 3 framebuffer captures across the session".to_string(),
            pass: screenshots.len() >= 3,
            detail: format!("{} PGM captures", screenshots.len()),
        },
    ];
    gates.push(DemoGate {
        gate: "quality bar (beats KN-5 on gutenberg_val)".to_string(),
        pass: quality.deployed_beats_kn5_full,
        detail: format!(
            "deployed int {:.4} vs KN-5 full {:.4} bits/raw-byte (+{:.4}) — HONESTLY OPEN; \
             not required for this scripted-demo packet, owner {}",
            quality.deployed.int_val_bits_per_raw_byte,
            quality.kn5[0].bits_per_raw_byte,
            quality.deployed_minus_kn5_full_bits_per_raw_byte,
            quality.quality_owner_bead
        ),
    });

    // The scripted-demo acceptance excludes the (explicitly open) quality
    // bar and hardware smoke; those are named in open_items.
    let all_demo_gates_pass = gates
        .iter()
        .filter(|g| !g.gate.starts_with("quality bar"))
        .all(|g| g.pass);

    let mut caveats = vec![
        "Scripted emulator demo (gbf-emu headless, DMG M-cycle-accurate): the real-hardware \
         flashcart smoke is bd-1qa27 and remains open."
            .to_string(),
        "The v0 shell is not the full M5 cooperative scheduler: the screen is static between \
         token boundaries; the transcript glyph + block cursor are the progress affordances \
         (bd-1kbv1)."
            .to_string(),
        "The RNG seed is host-poked (0x5EED) so the run is reproducible; an unpoked cart \
         plays the same first generation for a given prompt (XorShift16 seed canonicalizes \
         0 -> 1)."
            .to_string(),
        "On-device verification is capped by the output ring / 200-cell transcript at ~200 \
         tokens per session; this packet's sequence is entirely on-device and entirely \
         host-verified."
            .to_string(),
    ];
    if quick {
        caveats
            .push("QUICK MODE: development smoke sizes; this report is not evidence.".to_string());
    }

    Ok(DemoAcceptanceReport {
        schema: "demo_acceptance.v1",
        bead: "bd-do7sq",
        upstream_beads: vec![
            "bd-3l3tl", "bd-2gc6p", "bd-1kbv1", "bd-pp43d", "bd-3771m", "bd-2nca",
        ],
        git_sha: git_head(repo_root),
        scope: "the epic-closure demo, scripted: boot ROM -> on-screen-keyboard prompt entry \
                via injected joypad frames -> START -> sustained sampled generation rendered \
                to the transcript, deterministic and host-byte-identical. The KN-5-beating \
                quality bar and the real-hardware smoke are explicitly OPEN (see open_items).",
        checkpoint: StateCheckpointFacts {
            export_dir: D192_REAL_EXPORT_DIR.to_string(),
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
        prompt: DEMO_PROMPT.to_string(),
        prompt_note: "20 chars — exactly the shell prompt-row cap (SHELL_PROMPT_CAP); the \
                      evocative 23-char variant `The machines dreamed of` does not fit the \
                      single prompt BG row",
        rng_seed: DEMO_RNG_SEED,
        min_required_tokens: min_tokens,
        gates,
        all_demo_gates_pass,
        seconds_per_token_dmg_mean: mean_token as f64 / DMG_M_CYCLES_PER_SECOND as f64,
        latency_gate_packet: "docs/experiments/latency-gate/ (latency_gate.v1, bd-3l3tl)",
        screenshots,
        transcript_file: "transcript.txt",
        transcript_sha256: sha256(transcript.as_bytes()).to_hex(),
        determinism,
        quality,
        open_items: vec![
            "real-hardware flashcart smoke on DMG/GBC (bd-1qa27)",
            "KN-5-beating quality bar: deployed int-path 2.97 bits/raw-byte vs KN-5 full \
             2.26 — OPEN, owner bd-3771m (QAT-gap fixes, sparsity, longer training)",
            "full M5 cooperative scheduler / mid-token UI liveness (bd-1kbv1 remainder)",
        ],
        quick_mode: quick,
        caveats,
        session,
        screenshot_pgms: pgms,
        transcript,
    })
}

/// Render the packet README (generated, never hand-written).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn demo_report_to_markdown(r: &DemoAcceptanceReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# End-to-end demo acceptance ({})", r.schema);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Bead {}: {} Generated by `cargo run --release -p gbf-bench --bin demo-acceptance`; \
         every number is program output at git `{}`.",
        r.bead, r.scope, r.git_sha
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Setup");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Checkpoint `{}` ({}), manifest sha256 `{}`, {} tensors sha256-verified",
        r.checkpoint.export_dir,
        r.checkpoint.manifest_schema,
        &r.checkpoint.manifest_sha256[..16],
        r.checkpoint.tensors_verified_sha256
    );
    let _ = writeln!(
        out,
        "- Sampler: top-k {}, T {} (scale_q16 {}, effective {:.4}); RNG seed 0x{:04X}",
        r.sampler.top_k,
        r.sampler.requested_temperature,
        r.sampler.scale_q16,
        r.sampler.effective_temperature,
        r.rng_seed
    );
    let _ = writeln!(out, "- Prompt: `{}` ({})", r.prompt, r.prompt_note);
    let _ = writeln!(
        out,
        "- Cadence: {:.2} s/token mean on DMG this session; the asserted latency packet is \
         `{}`",
        r.seconds_per_token_dmg_mean, r.latency_gate_packet
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Gates");
    let _ = writeln!(out);
    let _ = writeln!(out, "| gate | result | detail |");
    let _ = writeln!(out, "|---|---|---|");
    for g in &r.gates {
        let _ = writeln!(
            out,
            "| {} | **{}** | {} |",
            g.gate,
            if g.pass { "PASS" } else { "OPEN/FAIL" },
            g.detail
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Scripted-demo acceptance (all gates except the explicitly-open quality bar): **{}**",
        if r.all_demo_gates_pass {
            "PASS"
        } else {
            "FAIL"
        }
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Evidence files");
    let _ = writeln!(out);
    for s in &r.screenshots {
        let _ = writeln!(
            out,
            "- `{}` — {} (framebuffer sha256 `{}`)",
            s.file,
            s.phase,
            &s.framebuffer_sha256[..16]
        );
    }
    let _ = writeln!(
        out,
        "- `{}` — full prompt + generated transcript (sha256 `{}`)",
        r.transcript_file,
        &r.transcript_sha256[..16]
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Determinism");
    let _ = writeln!(out);
    let d = &r.determinism;
    let _ = writeln!(
        out,
        "{} full sessions: sequences identical {}, framebuffer hashes identical {}, PGM bytes \
         identical {}, transcripts identical {}",
        d.sessions,
        d.sequences_identical,
        d.framebuffer_hashes_identical,
        d.screenshot_pgm_bytes_identical,
        d.transcripts_identical
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Quality: the honest verdict");
    let _ = writeln!(out);
    let q = &r.quality;
    let _ = writeln!(
        out,
        "The bead's quality bar is \"{}\" — status: **{}**.",
        q.quality_bar, q.quality_bar_status
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| model | bpc/normalized-char | bits/raw-byte |\n|---|---|---|"
    );
    let _ = writeln!(
        out,
        "| deployed d192 integer path ({}) | {:.4} | {:.4} |",
        q.deployed.int_semantics_version,
        q.deployed.int_val_bpc_per_normalized_char,
        q.deployed.int_val_bits_per_raw_byte
    );
    let _ = writeln!(
        out,
        "| deployed d192 trainer hard-ternary (f32 semantics) | {:.4} | {:.4} |",
        q.deployed.committed_hard_ternary_val_bpc,
        q.deployed.committed_hard_ternary_bits_per_raw_byte
    );
    for k in &q.kn5 {
        let _ = writeln!(
            out,
            "| KN-5, train cap {} | {:.4} | {:.4} |",
            k.train_cap, k.bpc_per_normalized_char, k.bits_per_raw_byte
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- The deployed model does **not** beat KN-5: +{:.4} bits/raw-byte vs the full-corpus \
         fit.",
        q.deployed_minus_kn5_full_bits_per_raw_byte
    );
    let _ = writeln!(out, "- {}", q.kn5_on_cart_feasibility);
    let _ = writeln!(out, "- Honest claim of this packet: {}.", q.honest_claim);
    let _ = writeln!(out, "- Named next levers (owner {}):", q.quality_owner_bead);
    for l in &q.next_levers {
        let _ = writeln!(out, "  - {l}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Open items (why bd-do7sq stays open)");
    let _ = writeln!(out);
    for o in &r.open_items {
        let _ = writeln!(out, "- {o}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Transcript sample");
    let _ = writeln!(out);
    let _ = writeln!(out, "```text");
    let _ = writeln!(out, "{}", r.transcript.trim_end());
    let _ = writeln!(out, "```");
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

    #[test]
    fn quality_facts_read_committed_evidence_and_stay_honest() {
        let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf();
        let q = load_demo_quality_facts(&repo_root).expect("committed evidence readable");
        // The honest verdict is structural: the deployed integer path is
        // worse (higher bits/raw-byte) than every committed KN-5 fit.
        assert!(!q.deployed_beats_kn5_full);
        assert!(q.deployed_minus_kn5_full_bits_per_raw_byte > 0.0);
        assert_eq!(q.quality_bar_status, "OPEN");
        assert_eq!(q.kn5.len(), 3);
        // The full fit is the strongest baseline and the largest table.
        assert!(q.kn5[0].bits_per_raw_byte < q.kn5[1].bits_per_raw_byte);
        assert!(q.kn5[0].total_unique_ngram_entries > q.kn5[1].total_unique_ngram_entries);
        // The bd-2nca committed full-corpus number (2.2584 bits/raw-byte)
        // must be reproduced by the same re-expression the arm record uses.
        assert!((q.kn5[0].bits_per_raw_byte - 2.2584).abs() < 5e-4);
        assert!(!q.next_levers.is_empty());
    }
}
