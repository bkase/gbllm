//! REAL d192 checkpoint bring-up evidence runner (bd-pp43d): the committed
//! S8 distilled student export -> production loader -> banked ROM ->
//! byte-exact emulator gates -> full-stream fidelity -> shell session ->
//! sampled text, written to `docs/experiments/d192-real/`.
//!
//! Usage: `cargo run --release -p gbf-bench --bin d192-real [-- quick]`
//! (`quick` runs development smoke sizes and marks the report as such).

use std::fs;
use std::path::PathBuf;

use gbf_bench::d192_real::{
    D192RealOptions, d192_real_report_to_markdown, run_d192_real_bringup, run_d192_real_v2_parity,
};
use gbf_bench::shell::framebuffer_to_pgm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let quick = std::env::args().any(|a| a == "quick");
    let opts = if quick {
        D192RealOptions::quick()
    } else {
        D192RealOptions::full()
    };

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let out_dir = repo_root.join("docs/experiments/d192-real");
    fs::create_dir_all(&out_dir)?;

    eprintln!(
        "running REAL d192 bring-up ({} mode; emulated ROM runs; full fidelity takes minutes)...",
        if quick { "quick" } else { "full" }
    );
    let report = run_d192_real_bringup(&repo_root, &opts);

    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        out_dir.join("README.md"),
        d192_real_report_to_markdown(&report),
    )?;
    if let Some(shell) = &report.shell {
        for (name, fb) in &shell.framebuffers {
            fs::write(out_dir.join(name), framebuffer_to_pgm(fb))?;
        }
        fs::write(out_dir.join("transcript.txt"), &shell.transcript)?;
    }
    for sample in &report.samples {
        fs::write(
            out_dir.join(&sample.file),
            format!("{}{}", sample.prompt, sample.text),
        )?;
    }

    for phase in &report.phases {
        eprintln!(
            "  {:24} {}{}",
            phase.phase,
            phase.status.to_uppercase(),
            phase
                .error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
    }
    // Step 5: V2 dispatch parity on the real checkpoint (byte-exact vs the
    // committed integer semantics + cycle/capacity accounting). Heavy: emulates
    // the real ~400-bank ROM under both lowerings.
    eprintln!("running V2 dispatch parity on the REAL checkpoint (emulates both lowerings)...");
    let v2 = run_d192_real_v2_parity(&repo_root)?;
    fs::write(
        out_dir.join("v2_parity.json"),
        serde_json::to_vec_pretty(&v2)?,
    )?;
    eprintln!(
        "d192-real V2 parity: one-token {} | generation {} | checkpoints {} | \
         V2 {} banks / {:.2} MiB (V3 {} banks / {:.2} MiB) | V2 {:.1} M-cyc/token \
         ({:.2}x V3, {:.1} s/token) | sampling fits {} ({} B) | shell fits {} ({} B)",
        v2.one_token_byte_exact,
        v2.multi_token_sequences_match,
        v2.multi_token_checkpoints_byte_exact,
        v2.v2_bank_count,
        v2.v2_rom_mib,
        v2.v3_bank_count,
        v2.v3_rom_mib,
        v2.v2_mean_m_cycles as f64 / 1.0e6,
        v2.v2_over_v3_cycles,
        v2.v2_seconds_per_token_dmg,
        v2.v2_sampling_fits_bank0,
        v2.v2_sampling_driver_bytes,
        v2.v2_shell_fits_bank0,
        v2.v2_shell_driver_bytes,
    );

    eprintln!("evidence: {}", out_dir.display());
    if !report.all_gates_pass {
        return Err("d192 real bring-up gates FAILED (see report)".into());
    }
    if !v2.pass() {
        return Err("d192-real V2 dispatch parity gate FAILED".into());
    }
    Ok(())
}
