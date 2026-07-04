//! `gbf compile` pipeline gate evidence runner (bd-1skgm).
//!
//! Usage: `cargo run --release -p gbf-bench --bin compile-gate -- [repo_root]`
//! Compiles the committed checkpoint export through the full compiler
//! dataflow, runs the 4-seed 256-token generation gate against the host
//! integer evaluator (plus the committed bd-2gc6p evidence cross-check), and
//! writes `docs/experiments/gbf-compile/report.json` + `README.md` and one
//! `sample_seed_0x<b>.txt` per seed. Every number and byte in the output is
//! produced by this program from the committed checkpoint export and live
//! emulator runs. Exits nonzero if the gate fails.

use gbf_bench::compile_gate::{compile_gate_report_to_markdown, run_compile_gate};
use gbf_bench::multi_token::{GENERATION_SEEDS, GENERATION_TOKENS, render_sample_text};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let report = run_compile_gate(
        &repo_root,
        "experiments/S6/checkpoint-export",
        &GENERATION_SEEDS,
        GENERATION_TOKENS,
    )?;

    let out_dir = repo_root.join("docs/experiments/gbf-compile");
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
    let markdown = compile_gate_report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    if !report.gate_passes() {
        eprintln!("gbf compile pipeline gate FAILED");
        std::process::exit(1);
    }
    Ok(())
}
