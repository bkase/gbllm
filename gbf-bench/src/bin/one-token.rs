//! One-token bring-up evidence runner (bd-59qiq).
//!
//! Usage: `cargo run --release -p gbf-bench --bin one-token -- [repo_root] [max_pairs]`
//! Writes `docs/experiments/one-token/report.json` and `README.md`.
//! Every number in the output is produced by this program from the committed
//! checkpoint export, the committed corpus, and live emulator runs.

use gbf_bench::one_token::{report_to_markdown, run_one_token_bringup};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let max_pairs: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(262_144);

    let report = run_one_token_bringup(
        &repo_root,
        "experiments/S6/checkpoint-export",
        1024 * 1024,
        max_pairs,
    )?;

    let out_dir = repo_root.join("docs/experiments/one-token");
    std::fs::create_dir_all(&out_dir)?;
    let json_path = out_dir.join("report.json");
    let md_path = out_dir.join("README.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    let markdown = report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    if !report.rom_gate.all_byte_exact {
        eprintln!("ROM agreement gate FAILED");
        std::process::exit(1);
    }
    Ok(())
}
