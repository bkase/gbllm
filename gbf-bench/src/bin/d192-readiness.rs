//! D192 readiness evidence generator (bd-pp43d): synthetic
//! d192/ff384/6blk/slots192 export -> production loader -> banked ROM ->
//! byte-exact emulator gates -> `docs/experiments/d192-readiness/`.
//!
//! Usage: `cargo run --release -p gbf-bench --bin d192-readiness`

use std::fs;
use std::path::PathBuf;

use gbf_bench::d192::{d192_report_to_markdown, run_d192_readiness};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let out_dir = repo_root.join("docs/experiments/d192-readiness");
    fs::create_dir_all(&out_dir)?;
    let work_dir = std::env::temp_dir().join("gbf-d192-readiness");
    let _ = fs::remove_dir_all(&work_dir);

    eprintln!("running d192 readiness gates (synthetic export; emulated ROM runs)...");
    let report = run_d192_readiness(&work_dir)?;

    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(out_dir.join("README.md"), d192_report_to_markdown(&report))?;
    let _ = fs::remove_dir_all(&work_dir);

    let pass = report.one_token_gate.all_byte_exact
        && report.multi_token.all_sequences_match
        && report.multi_token.all_health_checks_pass;
    eprintln!(
        "d192 readiness: one-token {} | multi-token sequences {} | health {} | {:.3} s/token",
        report.one_token_gate.all_byte_exact,
        report.multi_token.all_sequences_match,
        report.multi_token.all_health_checks_pass,
        report.cycles.seconds_per_token_dmg
    );
    eprintln!("evidence: {}", out_dir.display());
    if !pass {
        return Err("d192 readiness gates FAILED".into());
    }
    Ok(())
}
