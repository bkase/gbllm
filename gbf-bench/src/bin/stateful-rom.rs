//! Stateful ROM bring-up evidence runner (bd-x5l2s).
//!
//! Usage: `cargo run --release -p gbf-bench --bin stateful-rom -- [repo_root]
//! [max_positions_per_lane]`
//!
//! Writes `docs/experiments/stateful-rom/report.json`, `README.md`, and one
//! `sample_seed_*.txt` per seed — the first on-device stateful generation.
//! Every number and byte in the output is produced by this program from the
//! committed arm-B checkpoint export and live emulator runs.
//! `max_positions_per_lane` 0 (default) scores the full val stream,
//! reproducing the committed S5 A/B pair set exactly.

use gbf_bench::stateful::{
    STATE_EXPORT_DIR, render_char_sample, run_stateful_bringup, stateful_report_to_markdown,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let max_positions_per_lane: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(0);

    let report = run_stateful_bringup(&repo_root, STATE_EXPORT_DIR, max_positions_per_lane)?;

    let out_dir = repo_root.join("docs/experiments/stateful-rom");
    std::fs::create_dir_all(&out_dir)?;
    for run in &report.multi_token.runs {
        std::fs::write(
            out_dir.join(&run.sample_file),
            render_char_sample(&run.rom_sequence),
        )?;
    }
    let json_path = out_dir.join("report.json");
    let md_path = out_dir.join("README.md");
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    let markdown = stateful_report_to_markdown(&report);
    std::fs::write(&md_path, &markdown)?;
    println!("{markdown}");
    println!("wrote {} and {}", json_path.display(), md_path.display());

    let fidelity_ok = report
        .fidelity
        .f32_port_reproduces_committed_within_1e3
        .unwrap_or(false);
    if !fidelity_ok {
        eprintln!("stateful-rom: f32 port did NOT reproduce the committed arm-B bpc");
    }
    if !report.one_token_gate.all_byte_exact {
        eprintln!("stateful-rom: one-token gate FAILED");
    }
    if !report.multi_token.all_sequences_match || !report.multi_token.all_health_checks_pass {
        eprintln!("stateful-rom: multi-token generation gate FAILED");
    }
    if !fidelity_ok
        || !report.one_token_gate.all_byte_exact
        || !report.multi_token.all_sequences_match
        || !report.multi_token.all_health_checks_pass
    {
        std::process::exit(1);
    }
    Ok(())
}
