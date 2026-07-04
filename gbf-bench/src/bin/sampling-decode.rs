//! Sampling decode bring-up evidence runner (bd-2mjkd).
//!
//! Usage: `cargo run --release -p gbf-bench --bin sampling-decode -- [repo_root]`
//!
//! Writes `docs/experiments/sampling-decode/report.json`, `README.md`, one
//! `sample_rom_*.txt` per ROM gate combo, and one `sample_T*_k*_*.txt` per
//! qualitative setting. Every number and byte in the output is produced by
//! this program from the committed arm-B checkpoint export, the host
//! integer sampler, and live emulator runs.

use gbf_bench::sampling::{run_sampling_bringup, sampling_report_to_markdown};
use gbf_bench::stateful::{STATE_EXPORT_DIR, render_char_sample};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let report = run_sampling_bringup(&repo_root, STATE_EXPORT_DIR)?;

    let out_dir = repo_root.join("docs/experiments/sampling-decode");
    std::fs::create_dir_all(&out_dir)?;
    for run in &report.gate.runs {
        std::fs::write(
            out_dir.join(&run.sample_file),
            render_char_sample(&run.rom_sequence),
        )?;
    }
    for sample in &report.samples {
        std::fs::write(out_dir.join(&sample.file), &sample.text)?;
    }
    let json_path = out_dir.join("report.json");
    let md_path = out_dir.join("README.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    let markdown = sampling_report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    if !report.gate.all_sequences_match {
        eprintln!("sampling-decode: ROM/host sequence gate FAILED");
    }
    if !report.gate.all_health_checks_pass {
        eprintln!("sampling-decode: health checks FAILED");
    }
    if !report.gate.all_sequences_match || !report.gate.all_health_checks_pass {
        std::process::exit(1);
    }
    Ok(())
}
