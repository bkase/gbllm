//! Latency budget gate evidence runner (bd-3l3tl): sustained-generation
//! latency of the deployed d192 shell ROM over a real scripted session,
//! asserted against the 30 s/char UX budget, written to
//! `docs/experiments/latency-gate/`.
//!
//! Usage: `cargo run --release -p gbf-bench --bin latency-gate [-- quick]`
//! (`quick` runs development smoke sizes, marks the report as such, and
//! writes to a `latency-gate-quick` sibling directory instead).

use std::fs;
use std::path::PathBuf;

use gbf_bench::latency::{latency_report_to_markdown, run_latency_gate};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let quick = std::env::args().any(|a| a == "quick");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let out_dir = if quick {
        std::env::temp_dir().join("gbf-latency-gate-quick")
    } else {
        repo_root.join("docs/experiments/latency-gate")
    };
    fs::create_dir_all(&out_dir)?;

    eprintln!(
        "running latency gate ({} mode; two full scripted shell sessions)...",
        if quick { "quick" } else { "full" }
    );
    let report = run_latency_gate(&repo_root, quick)?;

    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(
        out_dir.join("README.md"),
        latency_report_to_markdown(&report),
    )?;

    for row in &report.rows {
        eprintln!(
            "  {:9} {:60} tokens {:3}  p50 {:.3}  p95 {:.3}  max {:.3} s/char  idle {:.4} Hz  {}",
            row.role,
            row.model,
            row.tokens.n_tokens,
            row.tokens.p50_s_per_char,
            row.tokens.p95_s_per_char,
            row.tokens.max_s_per_char,
            row.idle_poll.measured_hz,
            if row.row_pass { "PASS" } else { "FAIL" }
        );
    }
    eprintln!("evidence: {}", out_dir.display());
    if !report.gate_pass {
        return Err("latency gate FAILED (see report)".into());
    }
    Ok(())
}
