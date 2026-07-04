//! Interactive generation shell evidence runner (bd-1kbv1, v0 demo scope).
//!
//! Usage: `cargo run --release -p gbf-bench --bin interactive-shell --
//! [repo_root] [n_gen_tokens]`
//!
//! Builds the playable shell ROM from the committed arm-B checkpoint, runs
//! the scripted joypad session twice (determinism), and writes
//! `docs/experiments/interactive-shell/{report.json, README.md,
//! screenshot_*.pgm, transcript.txt}` — every byte program-generated.

use gbf_bench::shell::{
    SHELL_EXPORT_DIR, framebuffer_to_pgm, run_shell_bringup, shell_report_to_markdown,
    transcript_text,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let n_gen_tokens: u8 = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(200);

    let report = run_shell_bringup(&repo_root, SHELL_EXPORT_DIR, n_gen_tokens)?;

    let out_dir = repo_root.join("docs/experiments/interactive-shell");
    std::fs::create_dir_all(&out_dir)?;
    for (name, fb) in &report.session.framebuffers {
        std::fs::write(out_dir.join(name), framebuffer_to_pgm(fb))?;
    }
    std::fs::write(
        out_dir.join("transcript.txt"),
        transcript_text(&report.session.prompt_ids, &report.session.rom_sequence),
    )?;
    let json_path = out_dir.join("report.json");
    let md_path = out_dir.join("README.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    let markdown = shell_report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    let ok = report.session.all_gates_pass()
        && report.determinism.sequences_identical
        && report.determinism.framebuffer_hashes_identical;
    if !ok {
        eprintln!("interactive-shell: gate FAILED");
        std::process::exit(1);
    }
    Ok(())
}
