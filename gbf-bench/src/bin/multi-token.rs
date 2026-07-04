//! Multi-token sustained generation evidence runner (bd-2gc6p).
//!
//! Usage: `cargo run --release -p gbf-bench --bin multi-token -- [repo_root]`
//! Writes `docs/experiments/multi-token/report.json`, `README.md`, and one
//! `sample_seed_0x<b>.txt` per seed (the first on-device-generated text).
//! Every number and byte in the output is produced by this program from the
//! committed checkpoint export and live emulator runs.

use gbf_bench::multi_token::{
    GENERATION_SEEDS, GENERATION_TOKENS, multi_report_to_markdown, render_sample_text,
    run_multi_token_generation,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let report = run_multi_token_generation(
        &repo_root,
        "experiments/S6/checkpoint-export",
        &GENERATION_SEEDS,
        GENERATION_TOKENS,
    )?;

    let out_dir = repo_root.join("docs/experiments/multi-token");
    std::fs::create_dir_all(&out_dir)?;
    for run in &report.runs {
        std::fs::write(
            out_dir.join(&run.sample_file),
            render_sample_text(&run.rom_sequence),
        )?;
    }
    let json_path = out_dir.join("report.json");
    let md_path = out_dir.join("README.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    let markdown = multi_report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    if !report.all_sequences_match || !report.all_health_checks_pass {
        eprintln!("multi-token generation gate FAILED");
        std::process::exit(1);
    }
    Ok(())
}
